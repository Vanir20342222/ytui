//! Upstream `FFmpegFixupStretchedPP` for a loaded info dict's
//! `stretched_ratio`, with a recording FFmpeg.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let body = b"\0\0\0\x18ftypisom media";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

fn run(directory: &Path, tools: &Path, ratio: serde_json::Value, args: &[&str]) -> Output {
    let origin = serve();
    let info = serde_json::json!({
        "id": "vid", "title": "Vid", "ext": "mp4", "stretched_ratio": ratio,
        "webpage_url": format!("{origin}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": [{"format_id": "0", "url": format!("{origin}/v.mp4"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
    });
    let path = directory.join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .arg("--ffmpeg-location")
        .arg(tools)
        .arg("--load-info-json")
        .arg(&path)
        .args(args)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

#[test]
fn non_uniform_pixel_ratios_are_fixed_or_warned() {
    let tools = tempfile::tempdir().unwrap();
    fake_ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), tools.path(), serde_json::json!(1.5), &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with(
        "[download] Download completed\n[FixupStretched] Fixing aspect ratio of \"o.mp4\"\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"fixed"
    );
    let args = std::fs::read_to_string(tools.path().join("args.txt")).unwrap();
    assert!(
        args.contains("\n-map\n0\n-dn\n-ignore_unknown\n-c\ncopy\n-aspect\n1.500000\n-f\nmp4\n"),
        "{args}"
    );
    // `--fixup warn` only warns; a ratio of 1 needs nothing.
    for (ratio, args, stderr) in [
        (
            serde_json::json!(2),
            &["--fixup", "warn"][..],
            "WARNING: vid: Non-uniform pixel ratio 2\n",
        ),
        (serde_json::json!(1), &[][..], ""),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = run(directory.path(), tools.path(), ratio, args);
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("FixupStretched"));
        assert_eq!(String::from_utf8_lossy(&output.stderr), stderr);
    }
}
