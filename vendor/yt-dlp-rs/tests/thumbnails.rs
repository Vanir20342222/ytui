//! HTML5 poster thumbnails: listing, `--write-(all-)thumbnail(s)`,
//! `--convert-thumbnails` and `--embed-thumbnail` through the CLI.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, mime, body) = match path.as_str() {
                    "/page.html" => (
                        200,
                        "text/html",
                        "<title>Poster</title><video poster='/poster.jpg' src='/media.mp4'></video>",
                    ),
                    "/missing.html" => (
                        200,
                        "text/html",
                        "<title>Gone</title><video poster='/nope.jpg' src='/media.mp4'></video>",
                    ),
                    "/webp.html" => (
                        200,
                        "text/html",
                        "<title>Webp</title><video poster='/cover.jpg' src='/media.mp4'></video>",
                    ),
                    "/real.html" => (
                        200,
                        "text/html",
                        "<title>Real</title><video poster='/real.png' src='/real.mp4'></video>",
                    ),
                    "/plain.html" => (
                        200,
                        "text/html",
                        "<title>Plain</title><video src='/media.mp4'></video>",
                    ),
                    "/media.mp4" => (200, "video/mp4", "media"),
                    "/poster.jpg" => (200, "image/jpeg", "jpeg"),
                    "/cover.jpg" => (200, "image/jpeg", "RIFF\0\0\0\0WEBPVP8 "),
                    "/real.mp4" => (200, "video/mp4", ""),
                    "/real.png" => (200, "image/png", ""),
                    _ => (404, "text/plain", "missing"),
                };
                let body: &[u8] = match path.as_str() {
                    "/real.mp4" => include_bytes!("fixtures/merge/video.mp4"),
                    "/real.png" => include_bytes!("fixtures/thumbnails/cover.png"),
                    _ => body.as_bytes(),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        Self {
            origin,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cli(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn list_thumbnails_prints_the_sanitized_poster_table_without_downloads() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = cli(
        &["--list-thumbnails", &server.url("/page.html")],
        directory.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    // Listing does not imply quiet: extraction screen lines precede the table.
    let listing = |output: &Output| -> String {
        text(&output.stdout)
            .lines()
            .filter(|line| !line.starts_with("[generic] "))
            .map(|line| format!("{line}\n"))
            .collect()
    };
    assert_eq!(
        listing(&output),
        format!(
            "[info] Available thumbnails for page-1:\nID Width   Height  URL\n0  unknown unknown {}\n",
            server.url("/poster.jpg")
        )
    );
    let output = cli(
        &["--list-thumbnails", &server.url("/plain.html")],
        directory.path(),
    );
    assert_eq!(listing(&output), "plain-1 has no thumbnails\n");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let json = cli(&["-j", &server.url("/page.html")], directory.path());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["thumbnail"], server.url("/poster.jpg"));
    assert_eq!(value["thumbnails"][0]["id"], "0");
}

#[test]
fn write_thumbnail_follows_upstream_messages_overwrites_and_flag_order() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let page = server.url("/page.html");
    let output = cli(
        &[
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &page,
        ],
        directory.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("[info] Downloading video thumbnail 0 ..."));
    assert!(text(&output.stdout).contains("[info] Writing video thumbnail 0 to: video.jpg"));
    assert_eq!(
        std::fs::read(directory.path().join("video.jpg")).unwrap(),
        b"jpeg"
    );
    assert!(!directory.path().join("video.mp4").exists());
    std::fs::write(directory.path().join("video.jpg"), b"old").unwrap();
    let output = cli(
        &[
            "-w",
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &page,
        ],
        directory.path(),
    );
    assert!(text(&output.stdout).contains("[info] Video thumbnail is already present"));
    assert_eq!(
        std::fs::read(directory.path().join("video.jpg")).unwrap(),
        b"old"
    );
    // By default the existing thumbnail is replaced.
    cli(
        &[
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &page,
        ],
        directory.path(),
    );
    assert_eq!(
        std::fs::read(directory.path().join("video.jpg")).unwrap(),
        b"jpeg"
    );
    // A missing thumbnail is reported, not fatal; the media still downloads.
    let missing = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--write-thumbnail",
            "-o",
            "video.%(ext)s",
            &server.url("/missing.html"),
        ],
        missing.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("[info] Video Thumbnail 0 does not exist"));
    assert!(missing.path().join("video.mp4").exists());
    assert!(!missing.path().join("video.jpg").exists());
    // --no-write-thumbnail after --write-thumbnail disables writing.
    let disabled = tempfile::tempdir().unwrap();
    cli(
        &[
            "--write-thumbnail",
            "--no-write-thumbnail",
            "--skip-download",
            &page,
        ],
        disabled.path(),
    );
    assert_eq!(std::fs::read_dir(disabled.path()).unwrap().count(), 0);
    // --write-all-thumbnails with a single thumbnail uses the plain name.
    let all = tempfile::tempdir().unwrap();
    cli(
        &[
            "--write-all-thumbnails",
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "v.%(ext)s",
            &page,
        ],
        all.path(),
    );
    assert_eq!(std::fs::read(all.path().join("v.jpg")).unwrap(), b"jpeg");
    let none = cli(
        &[
            "--write-thumbnail",
            "--skip-download",
            &server.url("/plain.html"),
        ],
        all.path(),
    );
    assert!(text(&none.stdout).contains("[info] There are no video thumbnails to download"));
}

/// Fake ffmpeg/ffprobe pair built from the shared video pipeline helper.
struct Tools {
    _directory: tempfile::TempDir,
    probe: PathBuf,
    ffmpeg: PathBuf,
}
impl Tools {
    fn new() -> Self {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        let helper = HELPER.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let binary = directory.join(format!("video-pipeline{}", std::env::consts::EXE_SUFFIX));
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/helpers/video_pipeline.rs")
                    )
                    .args(["--edition=2024", "-Dwarnings", "-o"])
                    .arg(&binary)
                    .status()
                    .unwrap()
                    .success()
            );
            binary
        });
        let directory = tempfile::tempdir().unwrap();
        let probe = directory
            .path()
            .join(format!("probe-aac{}", std::env::consts::EXE_SUFFIX));
        let ffmpeg = directory
            .path()
            .join(format!("ffmpeg-ok{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(helper, &probe).unwrap();
        std::fs::copy(helper, &ffmpeg).unwrap();
        Self {
            _directory: directory,
            probe,
            ffmpeg,
        }
    }
    fn cli(&self, args: &[&str], directory: &Path) -> Output {
        let mut all: Vec<&std::ffi::OsStr> = vec![
            "--ffmpeg-location".as_ref(),
            self.ffmpeg.as_ref(),
            "--ffprobe-location".as_ref(),
            self.probe.as_ref(),
        ];
        all.extend(args.iter().map(std::ffi::OsStr::new));
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(all)
            .current_dir(directory)
            .output()
            .unwrap()
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.ffmpeg.with_extension("log")).unwrap_or_default()
    }
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn embed_thumbnail_writes_embeds_and_deletes_unless_written_explicitly() {
    let server = Server::new();
    let tools = Tools::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/page.html");
    let output = tools.cli(
        &["--embed-thumbnail", "-o", "video.%(ext)s", &url],
        directory.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("[info] Writing video thumbnail 0 to: video.jpg"));
    assert!(text(&output.stdout).ends_with(
        "[EmbedThumbnail] Neither mutagen nor AtomicParsley was found. Falling back to ffmpeg\n\
         [EmbedThumbnail] ffmpeg: Adding thumbnail to \"video.mp4\"\n"
    ));
    // The fake ffmpeg writes "source video" for two inputs.
    assert_eq!(
        names(directory.path()),
        ["video.mp4"],
        "{}",
        text(&output.stdout)
    );
    assert_eq!(
        std::fs::read(directory.path().join("video.mp4")).unwrap(),
        b"source video"
    );
    let log = tools.log();
    assert!(
        log.contains("\n-f\nimage2\n-pattern_type\nnone\n-protocol_whitelist\nfile\n-i\n"),
        "{log}"
    );
    assert!(
        log.contains("-map\n0\n-dn\n-ignore_unknown\n-c\ncopy\n-map\n1\n-disposition:1\nattached_pic\n-movflags\n+faststart\n-f\nmp4\n"),
        "{log}"
    );
    // An explicit --write-thumbnail keeps the embedded thumbnail.
    let kept = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--write-thumbnail",
            "--embed-thumbnail",
            "-o",
            "video.%(ext)s",
            &url,
        ],
        kept.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(names(kept.path()), ["video.jpg", "video.mp4"]);
    // Without thumbnails nothing is embedded and no tool runs.
    let plain = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--embed-thumbnail",
            "-o",
            "video.%(ext)s",
            &server.url("/plain.html"),
        ],
        plain.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    // Reported when the processor runs, after the download.
    assert!(
        text(&output.stdout)
            // The progress line's padding depends on the measured speed.
            .ends_with("\n[EmbedThumbnail] There aren't any thumbnails to embed\n"),
        "{}",
        text(&output.stdout)
    );
    assert_eq!(
        std::fs::read(plain.path().join("video.mp4")).unwrap(),
        b"media"
    );
    // --no-embed-thumbnail (last) disables embedding and the implied write.
    let disabled = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--embed-thumbnail",
            "--no-embed-thumbnail",
            "-o",
            "video.%(ext)s",
            &url,
        ],
        disabled.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(names(disabled.path()), ["video.mp4"]);
    assert_eq!(
        std::fs::read(disabled.path().join("video.mp4")).unwrap(),
        b"media"
    );
}

#[test]
fn embed_thumbnail_fixes_webp_extensions_and_converts_them_to_png() {
    let server = Server::new();
    let tools = Tools::new();
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--write-thumbnail",
            "--embed-thumbnail",
            "-o",
            "video.%(ext)s",
            &server.url("/webp.html"),
        ],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{:?}\n{}\n{}\n{}",
        output.status,
        text(&output.stdout),
        text(&output.stderr),
        tools.log()
    );
    // The kept original is renamed to its real WebP extension; the PNG
    // conversion used for embedding is temporary.
    assert_eq!(names(directory.path()), ["video.mp4", "video.webp"]);
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(
            "\n[ThumbnailsConvertor] Correcting thumbnail \"video.jpg\" extension to webp\n\
             [ThumbnailsConvertor] Converting thumbnail \"video.webp\" to png\n"
        ),
        "{stdout}"
    );
    let log = tools.log();
    assert!(log.contains("-update\n1\n-movflags\n+faststart\n"), "{log}");
    assert!(log.contains("thumbnail.png\n-map\n0\n"), "{log}");
}

#[test]
fn convert_thumbnails_follows_mappings_and_keep_video() {
    let server = Server::new();
    let tools = Tools::new();
    let url = server.url("/page.html");
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--write-thumbnail",
            "--convert-thumbnails",
            "png",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &url,
        ],
        directory.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains("[ThumbnailsConvertor] Converting thumbnail \"video.jpg\" to png"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Deleting original file video.jpg (pass -k to keep)"),
        "{stdout}"
    );
    assert_eq!(names(directory.path()), ["video.png"]);
    assert!(tools.log().contains("-update\n1\n-movflags\n+faststart\n"));
    // -k keeps the original; a mapping without a match skips conversion.
    let kept = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--write-thumbnail",
            "--convert-thumbnails",
            "png",
            "-k",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &url,
        ],
        kept.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(!text(&output.stdout).contains("Deleting original file"));
    assert_eq!(names(kept.path()), ["video.jpg", "video.png"]);
    let skipped = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--write-thumbnail",
            "--convert-thumbnails",
            "webp>png",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &url,
        ],
        skipped.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).contains("[ThumbnailsConvertor] Not converting thumbnail \"video.jpg\"; could not find a mapping for jpg"),
        "{}",
        text(&output.stdout)
    );
    assert_eq!(names(skipped.path()), ["video.jpg"]);
    // Invalid formats fail before any request.
    let output = cli(&["--convert-thumbnails", "gif", &url], skipped.path());
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("invalid thumbnail format \"gif\" given"));
}

#[test]
fn optional_real_tools_embed_a_png_cover_into_mp4() {
    let Some(ffmpeg) = std::env::var_os("YT_DLP_RS_TEST_FFMPEG") else {
        return;
    };
    let Some(ffprobe) = std::env::var_os("YT_DLP_RS_TEST_FFPROBE") else {
        return;
    };
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command
        .arg("--ffmpeg-location")
        .arg(&ffmpeg)
        .arg("--ffprobe-location")
        .arg(&ffprobe)
        .args([
            "--embed-thumbnail",
            "-o",
            "video.%(ext)s",
            &server.url("/real.html"),
        ])
        .current_dir(directory.path());
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(names(directory.path()), ["video.mp4"]);
    let probe = Command::new(&ffprobe)
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(directory.path().join("video.mp4"))
        .output()
        .unwrap();
    let probe: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert!(probe["streams"].as_array().unwrap().iter().any(|stream| {
        stream["codec_name"] == "png" && stream["disposition"]["attached_pic"] == 1
    }));
}
