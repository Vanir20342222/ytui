use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{Error, StreamKind, YoutubeDl};

const MASTER: &str = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Alternative\",LANGUAGE=\"nb\",AUTOSELECT=YES,URI=\"alt.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Main, English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"main.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"b\",NAME=\"Other default\",DEFAULT=YES,AUTOSELECT=YES,URI=\"other.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"b\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1920x1080,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"a\"\nhigh.m3u8\n";
const MEDIA: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-KEY:METHOD=AES-128,URI=\"keys/data.key\",IV=0x1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nsegment.m4s\n#EXT-X-ENDLIST\n";

struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(extra: BTreeMap<String, String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (done, record) = (stop.clone(), requests.clone());
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
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let length = stream.read(&mut buffer).unwrap();
                    if length == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..length]);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let (body, mime) = if let Some(body) = extra.get(path) {
                    (body.as_str(), "application/vnd.apple.mpegurl")
                } else {
                    match path {
                        "/master.m3u8" | "/second.m3u8" => {
                            (MASTER, "application/vnd.apple.mpegurl")
                        }
                        "/page.html" => (
                            "<title>Audio groups</title><video><source src='/master.m3u8'><source src='/second.m3u8'></video>",
                            "text/html",
                        ),
                        "/alt.m3u8" | "/main.m3u8" | "/other.m3u8" | "/low.m3u8" | "/high.m3u8" => {
                            (MEDIA, "application/vnd.apple.mpegurl")
                        }
                        "/aac.m3u8" => (
                            "#EXTM3U\n#EXTINF:1,\nsegment.aac\n#EXT-X-ENDLIST\n",
                            "application/vnd.apple.mpegurl",
                        ),
                        _ => ("unexpected segment or key request", "text/plain"),
                    }
                };
                record.lock().unwrap().push(request.clone());
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body.as_bytes());
            }
        });
        Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
    fn assert_no_media(&self) {
        for request in self.requests.lock().unwrap().iter() {
            let path = request.split_whitespace().nth(1).unwrap();
            assert!(path.ends_with(".m3u8") || path.ends_with(".html"), "{path}");
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("accept-encoding: identity\r\n")
            );
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn external_audio_groups_build_typed_plans_and_choose_matching_default() {
    let server = Server::new(BTreeMap::new());
    let dl = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg("unused-explicit-ffmpeg");
    let info = dl.extract_info(&server.url("/master.m3u8")).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("1000+a-Main__English"));
    assert_eq!(info.requested_formats.len(), 2);
    assert_eq!(info.requested_formats[0].stream_kind, StreamKind::VideoOnly);
    assert_eq!(info.requested_formats[1].stream_kind, StreamKind::AudioOnly);
    assert_eq!(info.requested_formats[1].audio_group.as_deref(), Some("a"));
    assert_eq!(info.requested_formats[1].language.as_deref(), Some("en"));
    assert_eq!(
        info.requested_formats[1].format_note.as_deref(),
        // Pinned upstream appends the group's quality rank among groups.
        Some("Main, English, high")
    );
    assert!(info.requested_formats[1].audio_default);
    assert!(info.requested_formats[1].audio_autoselect);
    assert_eq!(info.requested_formats[1].ext, "mp4");
    assert_eq!(info.requested_formats[1].fragments.len(), 2);
    assert!(info.requested_formats[1].fragments[0].encryption.is_some());
    // Without a merger, upstream's `best/bestvideo+bestaudio` selects the
    // same pair (there is no complete format).
    let unmerged = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/master.m3u8"))
        .unwrap();
    assert_eq!(unmerged.format_id, info.format_id);
    let audio = dl
        .extract_info_with_format(&server.url("/master.m3u8"), Some("a-Alternative"))
        .unwrap();
    assert_eq!(audio.stream_kind, StreamKind::AudioOnly);
    assert_eq!(audio.ext, "mp4");
    assert_eq!(audio.fragments.len(), 2);
    server.assert_no_media();
}

#[test]
fn html_groups_are_namespaced_and_manifest_requests_preserve_page_referer() {
    let server = Server::new(BTreeMap::new());
    let info = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg("unused-explicit-ffmpeg")
        .extract_info(&server.url("/page.html"))
        .unwrap();
    assert_eq!(info.title, "Audio groups (1)");
    assert_eq!(info.formats.len(), 10);
    // Pinned upstream sorts the page's formats by quality, interleaving the
    // two manifests; each keeps its source-index group namespace.
    assert_eq!(
        info.formats
            .iter()
            .map(|format| format.format_id.as_str())
            .collect::<Vec<_>>(),
        [
            "hls-b-Other_default-0",
            "hls-b-Other_default-1",
            "hls-a-Alternative-0",
            "hls-a-Alternative-1",
            "hls-a-Main__English-0",
            "hls-a-Main__English-1",
            "hls-500-0",
            "hls-500-1",
            "hls-1000-0",
            "hls-1000-1",
        ]
    );
    assert_eq!(
        info.format_id.as_deref(),
        Some("hls-1000-1+hls-a-Main__English-1")
    );
    for format in &info.formats {
        let source = &format.format_id[format.format_id.len() - 1..];
        assert!(
            format
                .audio_group
                .as_ref()
                .unwrap()
                .starts_with(&format!("{source}-")),
            "{}",
            format.format_id
        );
    }
    assert_eq!(
        info.requested_formats[0].audio_group,
        info.requested_formats[1].audio_group
    );
    for request in server.requests.lock().unwrap().iter().skip(1) {
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("referer: {}\r\n", server.url("/page.html")))
        );
    }
    server.assert_no_media();
}

#[test]
fn audio_only_master_declarations_and_aac_are_selectable() {
    let audio_only = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Only audio\",DEFAULT=YES,URI=\"aac.m3u8\"\n";
    let server = Server::new(BTreeMap::from([(
        "/audio-only.m3u8".into(),
        audio_only.into(),
    )]));
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/audio-only.m3u8"))
        .unwrap();
    assert_eq!(info.format_id.as_deref(), Some("a-Only_audio"));
    assert_eq!(info.stream_kind, StreamKind::AudioOnly);
    // Upstream labels HLS `mp4`, even for packed AAC audio.
    assert_eq!(info.ext, "mp4");
    assert_eq!(info.fragments.len(), 1);
    server.assert_no_media();
}

#[test]
fn json_simulation_retains_audio_metadata_and_does_not_fetch_keys_or_segments() {
    let server = Server::new(BTreeMap::new());
    let directory = tempfile::tempdir().unwrap();
    // An existing placeholder enables merging; simulation never runs it.
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(&ffmpeg, "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--dump-json",
            "--simulate",
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            &server.url("/master.m3u8"),
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // The audio group is internal; upstream's rendition dicts lack it.
    assert!(info["requested_formats"][1].get("audio_group").is_none());
    assert_eq!(info["requested_formats"][1]["format_id"], "a-Main__English");
    assert_eq!(info["requested_formats"][1]["language"], "en");
    // Upstream: a variant with external audio gets acodec `none`, its
    // renditions vcodec `none` without a known audio codec.
    assert_eq!(info["requested_formats"][0]["vcodec"], "avc1.4");
    assert_eq!(info["requested_formats"][0]["acodec"], "none");
    assert_eq!(info["requested_formats"][1]["vcodec"], "none");
    assert!(info["requested_formats"][1].get("acodec").is_none());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    server.assert_no_media();
}

#[test]
fn invalid_audio_declarations_fail_before_child_or_media_requests() {
    for master in [
        MASTER.replace("DEFAULT=YES", "DEFAULT=maybe"),
        MASTER.replace("AUTOSELECT=YES", "AUTOSELECT=maybe"),
        MASTER.replace("NAME=\"Alternative\"", "NAME=\"Main, English\""),
        MASTER.replace("NAME=\"Alternative\"", "NAME=\"Alternative\",DEFAULT=YES"),
        MASTER.replace(",URI=\"alt.m3u8\"", ""),
        MASTER.replace("URI=\"alt.m3u8\"", "URI=\"file:///audio.m3u8\""),
    ] {
        let server = Server::new(BTreeMap::from([("/bad.m3u8".into(), master)]));
        assert!(matches!(
            YoutubeDl::new()
                .unwrap()
                .extract_info(&server.url("/bad.m3u8")),
            Err(Error::UnsupportedManifest(_))
        ));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn audio_only_variant_with_initialization_is_labeled_mp4_like_upstream() {
    let server = Server::new(BTreeMap::from([(
        "/audio-variant.m3u8".into(),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\nmain.m3u8\n".into(),
    )]));
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/audio-variant.m3u8"))
        .unwrap();
    assert_eq!(info.stream_kind, StreamKind::AudioOnly);
    assert_eq!(info.ext, "mp4");
    assert_eq!(info.fragments.len(), 2);
    server.assert_no_media();
}
