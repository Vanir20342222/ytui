//! Local HTTP subtitle embedding and publication, using portable Rust media tools.
use std::{
    io::{Read, Write},
    net::TcpListener,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{
    DownloadOptions, DownloadPhase, Error, PostprocessOptions, SubtitleEmbedding, SubtitleFile,
    YoutubeDl,
};

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        Self::with_body(b"source video")
    }
    fn with_body(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, done) = (requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let line = String::from_utf8(request)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                let path = line.split_whitespace().nth(1).unwrap();
                let body: &[u8] = match path {
                    "/ttml-page.html" => b"<video src='/media.mp4'><track kind='subtitles' srclang='en' src='/captions.ttml'></video>",
                    "/captions.ttml" => TTML,
                    "/caption-page.html" | "/one.html" | "/two.html" => b"<video src='/media.mp4'><track kind='subtitles' srclang='en' src='/captions.srt'></video>",
                    "/captions.srt" => SRT,
                    "/audio-page.html" => b"<video src='/master.m3u8' type='application/vnd.apple.mpegurl'></video>",
                    "/master.m3u8" => b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.42001e\"\nvideo.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\naudio.m3u8\n",
                    "/video.m3u8" | "/audio.m3u8" => b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nsegment.ts\n#EXT-X-ENDLIST\n",
                    _ => body,
                };
                let mime = if path.ends_with(".srt") {
                    "text/plain"
                } else if path.ends_with(".html") {
                    "text/html"
                } else if path.ends_with(".m3u8") {
                    "application/vnd.apple.mpegurl"
                } else if path.ends_with(".mp3") {
                    "audio/mpeg"
                } else if path.ends_with(".webm") {
                    "video/webm"
                } else if path.ends_with(".m4a") {
                    "audio/mp4"
                } else {
                    "video/mp4"
                };
                record.lock().unwrap().push(line);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, name: &str) -> String {
        format!("{}/{name}", self.url)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn helper() -> &'static Path {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap().keep();
        let binary = directory.join(format!("embed-pipeline{}", std::env::consts::EXE_SUFFIX));
        assert!(
            Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/embed_pipeline.rs"))
                .args(["--edition=2024", "-Dwarnings", "-o"])
                .arg(&binary)
                .status()
                .unwrap()
                .success()
        );
        binary
    })
}
struct Tools {
    _directory: tempfile::TempDir,
    probe: PathBuf,
    ffmpeg: PathBuf,
}
impl Tools {
    fn new(codec: &str, fail: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let probe = directory
            .path()
            .join(format!("probe-{codec}{}", std::env::consts::EXE_SUFFIX));
        let ffmpeg = directory.path().join(format!(
            "ffmpeg-{}{}",
            if fail { "fail" } else { "ok" },
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::copy(helper(), &probe).unwrap();
        std::fs::copy(helper(), &ffmpeg).unwrap();
        Self {
            _directory: directory,
            probe,
            ffmpeg,
        }
    }
    fn client(&self) -> YoutubeDl {
        YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(&self.ffmpeg)
            .with_ffprobe(&self.probe)
    }
    fn log(&self, path: &Path) -> String {
        std::fs::read_to_string(path.with_extension("log")).unwrap_or_default()
    }
    fn cli(&self, args: &[&str], directory: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(directory)
            .args(["--ffmpeg-location"])
            .arg(&self.ffmpeg)
            .arg("--ffprobe-location")
            .arg(&self.probe)
            .args(args)
            .output()
            .unwrap()
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn clean(path: &Path) {
    assert!(!std::fs::read_dir(path).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".yt-dlp-rs-")
    }));
}

const SRT: &[u8] = b"1\n00:00:01,250 --> 00:00:03,500\nHello world\n";
const TTML: &[u8] = b"<tt xmlns='http://www.w3.org/ns/ttml'><body><div><p begin='1.25s' end='3.5s'>Hello world</p></div></body></tt>";
fn embedding(path: &Path, ext: &str, keep: bool) -> PostprocessOptions {
    PostprocessOptions {
        existing: None,
        no_post_overwrites: false,
        modify_chapters: None,
        ignore_errors: false,
        thumbnail: None,
        metadata: false,
        chapters: false,
        infojson: None,
        fixup: None,
        embed: Some(SubtitleEmbedding {
            subtitles: vec![SubtitleFile {
                path: path.into(),
                ext: ext.into(),
                language: "en".into(),
                name: Some("English captions".into()),
            }],
            keep_subtitles: keep,
        }),
        ..Default::default()
    }
}

#[test]
fn library_embeds_language_name_and_keeps_captions_without_extra_media_copy() {
    let server = Server::new();
    for keep in [false, true] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let info = client.extract_info(&server.url("source.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let caption = directory.path().join("captions.srt");
        std::fs::write(&caption, SRT).unwrap();
        let mut plan = embedding(&caption, "srt", keep);
        plan.keep_video = keep;
        let result = client
            .download_processed_with_options(
                &info,
                directory.path().join("movie.mp4"),
                DownloadOptions::default(),
                &plan,
            )
            .unwrap();
        assert_eq!(std::fs::read(&result.path).unwrap(), b"embedded media");
        assert_eq!(caption.exists(), keep);
        assert!(!directory.path().join("movie.orig.mp4").exists());
        assert_eq!(result.kept_video, None);
        let log = tools.log(&tools.ffmpeg);
        assert!(log.contains("language=eng"), "{log}");
        assert!(log.contains("English captions"), "{log}");
        clean(directory.path());
    }
}
#[test]
fn empty_or_ineligible_embedding_is_lazy_and_preserves_subtitles() {
    let server = Server::new();
    for (media, ext, sub_ext) in [
        ("source.mp3", "mp3", "srt"),
        ("source.webm", "webm", "srt"),
        ("source.mp4", "mp4", "json"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let client = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(directory.path().join("missing"));
        let info = client.extract_info(&server.url(media)).unwrap();
        assert_eq!(info.ext, ext);
        let caption = directory.path().join(format!("captions.{sub_ext}"));
        std::fs::write(&caption, SRT).unwrap();
        let result = client
            .download_processed_with_options(
                &info,
                directory.path().join(media),
                DownloadOptions::default(),
                &embedding(&caption, sub_ext, false),
            )
            .unwrap();
        assert_eq!(std::fs::read(result.path).unwrap(), b"source video");
        assert!(caption.exists());
        assert!(!result.warnings.is_empty());
        clean(directory.path());
    }
}
#[test]
fn cli_embed_implies_manual_subtitles_and_explicit_write_retains_them() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for explicit in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let url = server.url("caption-page.html");
        let mut args = vec!["--embed-subs", "-o", "out.mp4"];
        if explicit {
            args.push("--write-subs");
        }
        args.push(&url);
        success(&tools.cli(&args, directory.path()));
        assert_eq!(
            std::fs::read(directory.path().join("out.mp4")).unwrap(),
            b"embedded media"
        );
        assert_eq!(directory.path().join("out.en.srt").exists(), explicit);
        clean(directory.path());
    }
}
#[test]
fn cli_readonly_modes_do_not_fetch_and_skip_download_keeps_subtitles_unembedded() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for mode in ["-j", "-s", "-F", "--list-subs"] {
        let directory = tempfile::tempdir().unwrap();
        success(&tools.cli(
            &[
                "--embed-subs",
                mode,
                "-o",
                "out.mp4",
                &server.url("caption-page.html"),
            ],
            directory.path(),
        ));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("/captions.srt "))
    );
    let directory = tempfile::tempdir().unwrap();
    success(&tools.cli(
        &[
            "--embed-subs",
            "--skip-download",
            "--download-archive",
            "archive",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.en.srt")).unwrap(),
        SRT
    );
    assert!(!directory.path().join("out.mp4").exists());
    assert!(!directory.path().join("archive").exists());
    assert!(tools.log(&tools.ffmpeg).is_empty());
}
#[test]
fn converted_sidecar_and_remux_run_before_embedding() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    success(&tools.cli(
        &[
            "--embed-subs",
            "--convert-subs",
            "vtt",
            "--remux-video",
            "mkv",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.mkv")).unwrap(),
        b"embedded media"
    );
    assert!(!directory.path().join("out.en.vtt").exists());
    assert!(!directory.path().join("out.en.srt").exists());
    assert!(!directory.path().join("out.mp4").exists());
    let log = tools.log(&tools.ffmpeg);
    assert_eq!(log.lines().filter(|arg| *arg == "-i").count(), 4);
    clean(directory.path());
}
#[test]
fn embed_failure_preserves_latest_media_and_subtitles_without_archive() {
    let server = Server::new();
    let tools = Tools::new("aac", true);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--embed-subs",
            "--remux-video",
            "mkv",
            "--download-archive",
            "archive",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("out.mkv")).unwrap(),
        b"converted video"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.en.srt")).unwrap(),
        SRT
    );
    assert!(!directory.path().join("archive").exists());
    clean(directory.path());
}
#[test]
fn cancellation_before_processing_or_publishing_retains_captions_and_publishes_no_media() {
    let server = Server::new();
    for phase in [DownloadPhase::Postprocessing, DownloadPhase::Publishing] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let info = client.extract_info(&server.url("source.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let caption = directory.path().join("captions.srt");
        std::fs::write(&caption, SRT).unwrap();
        let result = client.download_processed_with_progress(
            &info,
            directory.path().join("movie.mp4"),
            DownloadOptions::default(),
            &embedding(&caption, "srt", false),
            |progress| {
                if progress.phase == phase {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert!(matches!(result, Err(Error::DownloadCancelled)));
        assert_eq!(std::fs::read(caption).unwrap(), SRT);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
#[test]
fn publication_replaces_a_competing_output_like_upstream() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let caption = directory.path().join("captions.srt");
    std::fs::write(&caption, SRT).unwrap();
    let destination = directory.path().join("movie.mp4");
    let result = client.download_processed_with_progress(
        &info,
        &destination,
        DownloadOptions::default(),
        &embedding(&caption, "srt", false),
        |progress| {
            if progress.phase == DownloadPhase::Publishing {
                std::fs::write(&destination, b"competitor").unwrap();
            }
            ControlFlow::Continue(())
        },
    );
    // Upstream replaces existing processor outputs (`os.replace`), then
    // deletes the embedded captions.
    assert!(result.is_ok());
    assert_eq!(std::fs::read(destination).unwrap(), b"embedded media");
    assert!(!caption.exists());
    clean(directory.path());
}

#[test]
fn empty_embedding_skips_tools_and_cli_last_disable_prevents_implicit_caption_download() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let client = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(directory.path().join("missing"));
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let plan = PostprocessOptions {
        existing: None,
        no_post_overwrites: false,
        modify_chapters: None,
        ignore_errors: false,
        thumbnail: None,
        metadata: false,
        chapters: false,
        infojson: None,
        fixup: None,
        embed: Some(SubtitleEmbedding {
            subtitles: Vec::new(),
            keep_subtitles: false,
        }),
        ..Default::default()
    };
    let result = client
        .download_processed_with_options(
            &info,
            directory.path().join("movie.mp4"),
            DownloadOptions::default(),
            &plan,
        )
        .unwrap();
    assert_eq!(std::fs::read(result.path).unwrap(), b"source video");
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    success(&tools.cli(
        &[
            "--embed-subs",
            "--no-embed-subs",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"source video"
    );
    assert!(!directory.path().join("out.en.srt").exists());
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("/captions.srt "))
    );
}

#[test]
fn archive_cannot_alias_caption_that_would_be_deleted_after_embedding() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--embed-subs",
            "--download-archive",
            "out.en.srt",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("/captions.srt "))
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    assert!(tools.log(&tools.ffmpeg).is_empty());
}

#[test]
fn optional_real_embedding_adds_cues_and_expected_subtitle_codec_in_three_containers() {
    let (Some(ffmpeg), Some(ffprobe)) = (
        std::env::var_os("YT_DLP_RS_TEST_FFMPEG"),
        std::env::var_os("YT_DLP_RS_TEST_FFPROBE"),
    ) else {
        return;
    };
    for (container, expected) in [("mp4", "mov_text"), ("mkv", "subrip"), ("webm", "webvtt")] {
        let webm = container == "webm";
        let server = Server::with_body(if webm {
            include_bytes!("fixtures/merge/video.webm")
        } else {
            include_bytes!("fixtures/merge/video.mp4")
        });
        let client = YoutubeDl::new().unwrap().with_ffmpeg(&ffmpeg);
        let info = client
            .extract_info(&server.url(if webm { "source.webm" } else { "source.mp4" }))
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let ext = if webm { "vtt" } else { "srt" };
        let caption = directory.path().join(format!("captions.{ext}"));
        std::fs::write(
            &caption,
            if webm {
                b"WEBVTT\n\n00:01.250 --> 00:03.500\nHello world\n".as_slice()
            } else {
                SRT
            },
        )
        .unwrap();
        let mut plan = embedding(&caption, ext, false);
        if container == "mkv" {
            plan.video =
                Some(yt_dlp_rs::VideoOptions::new("mkv", yt_dlp_rs::VideoMode::Remux).unwrap());
        }
        let result = client
            .download_processed_with_options(
                &info,
                directory
                    .path()
                    .join(if webm { "movie.webm" } else { "movie.mp4" }),
                DownloadOptions::default(),
                &plan,
            )
            .unwrap();
        let probe = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,codec_name:stream_tags=language,title,handler_name",
                "-of",
                "json",
            ])
            .arg(&result.path)
            .output()
            .unwrap();
        success(&probe);
        let value: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let subtitles: Vec<_> = value["streams"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|stream| stream["codec_type"] == "subtitle")
            .collect();
        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0]["codec_name"], expected);
        assert_eq!(subtitles[0]["tags"]["language"], "eng");
        let cues = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&result.path)
            .args(["-map", "0:s:0", "-f", "srt", "-"])
            .output()
            .unwrap();
        success(&cues);
        assert!(String::from_utf8_lossy(&cues.stdout).contains("Hello world"));
        assert!(!caption.exists());
        clean(directory.path());
    }
}

#[test]
fn failed_embedding_continues_urls_preserves_captions_and_never_records_archive() {
    let server = Server::new();
    let tools = Tools::new("aac", true);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--embed-subs",
            "--download-archive",
            "archive",
            "-o",
            "%(id)s.%(ext)s",
            &server.url("one.html"),
            &server.url("two.html"),
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("ERROR:")
            .count(),
        2
    );
    for id in ["one-1", "two-1"] {
        assert_eq!(
            std::fs::read(directory.path().join(format!("{id}.mp4"))).unwrap(),
            b"source video"
        );
        assert_eq!(
            std::fs::read(directory.path().join(format!("{id}.en.srt"))).unwrap(),
            SRT
        );
    }
    assert!(!directory.path().join("archive").exists());
    clean(directory.path());
}

#[test]
fn explicit_write_retains_converted_sidecar_after_embedding_and_keep_retains_prior_media_stage() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    success(&tools.cli(
        &[
            "--embed-subs",
            "--write-subs",
            "--convert-subs",
            "vtt",
            "--remux-video",
            "mkv",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.en.vtt")).unwrap(),
        b"converted subtitle"
    );
    assert!(!directory.path().join("out.en.srt").exists());
    assert_eq!(
        std::fs::read(directory.path().join("out.mkv")).unwrap(),
        b"embedded media"
    );
    let directory = tempfile::tempdir().unwrap();
    success(&tools.cli(
        &[
            "--embed-subs",
            "--remux-video",
            "mkv",
            "-k",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"source video"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.mkv")).unwrap(),
        b"embedded media"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.en.srt")).unwrap(),
        SRT
    );
    assert!(!directory.path().join("out.orig.mkv").exists());
    clean(directory.path());
}

#[test]
fn existing_media_destination_refuses_before_download_or_caption_deletion() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let caption = directory.path().join("captions.srt");
    std::fs::write(&caption, SRT).unwrap();
    let destination = directory.path().join("movie.mp4");
    std::fs::write(&destination, b"existing").unwrap();
    let before = server.requests.lock().unwrap().len();
    assert!(
        client
            .download_processed_with_options(
                &info,
                &destination,
                DownloadOptions::default(),
                &embedding(&caption, "srt", false)
            )
            .is_err()
    );
    assert_eq!(server.requests.lock().unwrap().len(), before);
    assert_eq!(std::fs::read(destination).unwrap(), b"existing");
    assert_eq!(std::fs::read(caption).unwrap(), SRT);
    assert!(tools.log(&tools.ffmpeg).is_empty());
}

#[test]
fn oversized_caption_is_rejected_and_completed_media_and_caption_are_preserved() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let caption = directory.path().join("captions.srt");
    std::fs::File::create(&caption)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    let destination = directory.path().join("movie.mp4");
    let error = client
        .download_processed_with_options(
            &info,
            &destination,
            DownloadOptions::default(),
            &embedding(&caption, "srt", false),
        )
        .unwrap_err();
    assert!(error.to_string().contains("16 MiB"));
    assert_eq!(std::fs::read(destination).unwrap(), b"source video");
    assert_eq!(
        std::fs::metadata(caption).unwrap().len(),
        16 * 1024 * 1024 + 1
    );
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    clean(directory.path());
}

#[test]
fn changed_caption_at_publication_is_retained_and_reports_cleanup_failure_after_media_publish() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let caption = directory.path().join("captions.srt");
    std::fs::write(&caption, SRT).unwrap();
    let destination = directory.path().join("movie.mp4");
    let error = client
        .download_processed_with_progress(
            &info,
            &destination,
            DownloadOptions::default(),
            &embedding(&caption, "srt", false),
            |progress| {
                if progress.phase == DownloadPhase::Publishing {
                    std::fs::write(&caption, b"changed subtitle with different length").unwrap();
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("cleanup failed"), "{error}");
    assert_eq!(std::fs::read(destination).unwrap(), b"embedded media");
    assert_eq!(
        std::fs::read(caption).unwrap(),
        b"changed subtitle with different length"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    clean(directory.path());
}
