//! HLS `EXT-X-MEDIA:TYPE=SUBTITLES` renditions: upstream lists them as
//! `m3u8_native` WebVTT subtitles and HlsFD joins their segments. Outputs
//! were checked against pinned upstream with the same playlists.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

const MASTER: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"subs_en.m3u8\"\n\
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"Other\",URI=\"subs_und.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=200000,RESOLUTION=320x240,SUBTITLES=\"subs\"\nvideo.m3u8\n";
const VIDEO: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nv0.ts\n#EXT-X-ENDLIST\n";
const SUBS_EN: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nen0.vtt\n#EXTINF:2.0,\nen1.vtt\n#EXT-X-ENDLIST\n";
const SUBS_UND: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nund0.vtt\n#EXT-X-ENDLIST\n";
const EN0: &str = "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n1\n00:00:00.500 --> 00:00:01.500\nHello\n\n00:00:01.800 --> 00:00:02.000\nSpans\n";
const EN1: &str = "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:1080000,LOCAL:00:00:00.000\n\n00:00:00.000 --> 00:00:00.400\nSpans\n\nNOTE comment\n\n00:00:00.500 --> 00:00:01.000 align:start\nWorld\n";
const UND0: &str = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHallo\n";

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (mime, body): (&str, &[u8]) = match path.as_str() {
                "/master.m3u8" => ("application/vnd.apple.mpegurl", MASTER.as_bytes()),
                "/video.m3u8" => ("application/vnd.apple.mpegurl", VIDEO.as_bytes()),
                "/subs_en.m3u8" => ("application/vnd.apple.mpegurl", SUBS_EN.as_bytes()),
                "/subs_und.m3u8" => ("application/vnd.apple.mpegurl", SUBS_UND.as_bytes()),
                "/en0.vtt" => ("text/vtt", EN0.as_bytes()),
                "/en1.vtt" => ("text/vtt", EN1.as_bytes()),
                "/und0.vtt" => ("text/vtt", UND0.as_bytes()),
                _ => ("video/mp2t", b"segment"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

fn run(directory: &Path, origin: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress"])
        .args(args)
        .arg(format!("{origin}/master.m3u8"))
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn subtitle_renditions_are_listed_and_joined() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), &origin, &["--list-subs"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with(
        "[info] Available subtitles for master:\nLanguage Formats\nen       vtt\nund      vtt\n"
    ));
    let output = run(
        directory.path(),
        &origin,
        &[
            "-o",
            "o.%(ext)s",
            "--write-subs",
            "--sub-langs",
            "all",
            "--skip-download",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(
        "[info] Writing video subtitles to: o.en.vtt\n[hlsnative] Downloading m3u8 manifest\n\
         [hlsnative] Total fragments: 2\n[download] Destination: o.en.vtt\n[download] Download completed\n"
    ));
    // Cues move by each segment's X-TIMESTAMP-MAP; a cue continued in the
    // next segment is merged.
    assert_eq!(
        std::fs::read_to_string(directory.path().join("o.en.vtt")).unwrap(),
        "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:900000\n\n1\n00:00:00.500 --> 00:00:01.500\nHello\n\n\
         NOTE comment\n\n00:00:01.800 --> 00:00:02.400\nSpans\n\n00:00:02.500 --> 00:00:03.000 align:start\nWorld\n\n"
    );
    // A single segment is copied as it is.
    assert_eq!(
        std::fs::read_to_string(directory.path().join("o.und.vtt")).unwrap(),
        UND0
    );
}
