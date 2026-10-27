//! Upstream `FFmpegFixupDuplicateMoovPP` for single DASH format downloads
//! (every MPD format is `is_dash_periods`), with a recording FFmpeg.
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
<AdaptationSet mimeType="audio/mp4" contentType="audio" lang="en"><Representation id="a1" bandwidth="100" codecs="mp4a.40.2"><SegmentList duration="1" timescale="1"><Initialization sourceURL="ainit.mp4"/><SegmentURL media="a1.m4s"/></SegmentList></Representation></AdaptationSet></Period></MPD>"#;

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
         for a in \"$@\"; do last=\"$a\"; done\nprintf 'fixed' > \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(directory: &Path, tools: &Path, args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
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
fn single_dash_formats_get_the_duplicate_moov_fixup() {
    let url = format!("{}/v.mpd", serve());
    let tools = tempfile::tempdir().unwrap();
    fake_ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), tools.path(), &["-f", "bv"], &url);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.ends_with(
        "[download] Download completed\n[FixupDuplicateMoov] Fixing duplicate MOOV atoms of \"o.mp4\"\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"fixed"
    );
    let args = std::fs::read_to_string(tools.path().join("args.txt")).unwrap();
    assert!(
        args.contains("\n-map\n0\n-dn\n-ignore_unknown\n-c\ncopy\n-f\nmp4\n"),
        "{args}"
    );
    // --fixup never and merges skip it.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools.path(),
        &["-f", "bv", "--fixup", "never"],
        &url,
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("FixupDuplicateMoov"));
}

#[test]
fn fixups_run_before_skipped_processors_but_not_before_conversions() {
    let url = format!("{}/v.mpd", serve());
    let tools = tempfile::tempdir().unwrap();
    fake_ffmpeg(tools.path());
    // `-x` of an m4a needs no conversion; the download is still fixed.
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), tools.path(), &["-x"], &url);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(
            "[FixupM4a] Correcting container of \"o.m4a\"\n[FixupDuplicateMoov] Fixing duplicate MOOV atoms of \"o.m4a\"\n[ExtractAudio] Not converting audio o.m4a; the file is already in a common audio format\n"
        ),
        "{stdout}"
    );
    // A remux to another container is `postprocessed_by_ffmpeg`.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools.path(),
        &["-f", "bv", "--remux-video", "mkv"],
        &url,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[VideoRemuxer] Remuxing video from mp4 to mkv"),
        "{stdout}"
    );
    assert!(!stdout.contains("FixupDuplicateMoov"), "{stdout}");
}
