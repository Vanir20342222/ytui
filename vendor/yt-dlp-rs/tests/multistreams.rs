//! Upstream `--video-multistreams`/`--audio-multistreams`: merges of several
//! streams, mapped like `FFmpegMergerPP`, with a recording FFmpeg.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT2S" minBufferTime="PT1S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
<Period><AdaptationSet mimeType="video/mp4" contentType="video"><Representation id="v1" bandwidth="1000" width="640" height="360" codecs="avc1.4d401e"><SegmentList duration="1" timescale="1"><Initialization sourceURL="init.mp4"/><SegmentURL media="s1.m4s"/></SegmentList></Representation></AdaptationSet>
<AdaptationSet mimeType="audio/mp4" contentType="audio" lang="en"><Representation id="en" bandwidth="200" codecs="mp4a.40.2"><SegmentList duration="1" timescale="1"><Initialization sourceURL="ainit.mp4"/><SegmentURL media="a1.m4s"/></SegmentList></Representation></AdaptationSet>
<AdaptationSet mimeType="audio/mp4" contentType="audio" lang="de"><Representation id="de" bandwidth="100" codecs="mp4a.40.2"><SegmentList duration="1" timescale="1"><Initialization sourceURL="binit.mp4"/><SegmentURL media="b1.m4s"/></SegmentList></Representation></AdaptationSet></Period></MPD>"#;

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
            let (mime, body): (&str, &[u8]) = if path == "/v.mpd" {
                ("application/dash+xml", MPD.as_bytes())
            } else {
                ("video/mp4", b"\0\0\0\x18ftypisom segment")
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

fn fake_ffmpeg(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
         printf '%s\\n' \"$@\" > \"${0%/ffmpeg}/args.txt\"\n\
         for a in \"$@\"; do last=\"$a\"; done\nprintf 'merged' > \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(directory: &Path, tools: &Path, args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-progress",
            "--fixup",
            "never",
            "-o",
            "o.%(ext)s",
        ])
        .arg("--ffmpeg-location")
        .arg(tools)
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

#[test]
fn audio_multistreams_merge_every_selected_stream() {
    let url = format!("{}/v.mpd", serve());
    let tools = tempfile::tempdir().unwrap();
    fake_ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools.path(),
        &["--audio-multistreams", "-f", "bv+ba+ba.2"],
        &url,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("[info] v: Downloading 1 format(s): v1+en+de\n"),
        "{stdout}"
    );
    let args = std::fs::read_to_string(tools.path().join("args.txt")).unwrap();
    assert_eq!(args.matches("\n-i\n").count(), 3, "{args}");
    assert!(
        args.contains("\n-map\n0:v:0\n-map\n1:a:0\n-map\n2:a:0\n-c\ncopy\n"),
        "{args}"
    );
    // `get_compatible_ext` picks mkv for several audio streams, as upstream.
    assert!(
        stdout.contains("[Merger] Merging formats into \"o.mkv\"\n"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mkv")).unwrap(),
        b"merged"
    );
    // Without the option, `_merge` keeps the first audio stream.
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), tools.path(), &["-f", "bv+ba+ba.2"], &url);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Downloading 1 format(s): v1+en\n"),
        "{stdout}"
    );
    let args = std::fs::read_to_string(tools.path().join("args.txt")).unwrap();
    assert_eq!(args.matches("\n-i\n").count(), 2, "{args}");
}
