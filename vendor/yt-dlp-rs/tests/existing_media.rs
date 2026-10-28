//! Existing downloads (upstream `existing_video_file` and
//! `FileDownloader.download`): reported, and processed again in place.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
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
            let body = b"\0\0\0\x18ftypisom media bytes";
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

/// Writes `remuxed` to its last argument (the output file).
fn fake_ffmpeg(directory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
         for a in \"$@\"; do last=\"$a\"; done\nprintf 'remuxed' > \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn run(directory: &Path, args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-o", "o.%(ext)s"])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn an_existing_file_is_reported_finished() {
    let url = format!("{}/v.mp4", serve());
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("o.mp4"), b"old").unwrap();
    let output = run(directory.path(), &["--no-progress"], &url);
    assert!(stdout(&output).ends_with(
        "[download] o.mp4 has already been downloaded\n[download] Download completed\n"
    ));
    let output = run(directory.path(), &["--progress", "--newline"], &url);
    assert!(stdout(&output).ends_with(
        "[download] o.mp4 has already been downloaded\n[download] 100% of      3.00B\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"old"
    );
    // --force-overwrites deletes it (`report_file_delete`) and downloads again.
    let output = run(
        directory.path(),
        &["--no-progress", "--force-overwrites"],
        &url,
    );
    assert!(stdout(&output).contains("\nDeleting existing file o.mp4\n"));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"\0\0\0\x18ftypisom media bytes"
    );
}

#[test]
fn existing_files_are_processed_again() {
    let url = format!("{}/v.mp4", serve());
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let ffmpeg = ffmpeg.to_str().unwrap();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("o.mp4"), b"\0\0\0\x18ftypisom old").unwrap();
    let remux = [
        "--no-progress",
        "--ffmpeg-location",
        ffmpeg,
        "--remux-video",
        "mkv",
    ];
    let output = run(directory.path(), &[&remux[..], &["-k"]].concat(), &url);
    assert!(stdout(&output).ends_with(
        "[download] o.mp4 has already been downloaded\n[download] Download completed\n[VideoRemuxer] Remuxing video from mp4 to mkv; Destination: o.mkv\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.mkv")).unwrap(),
        b"remuxed"
    );
    assert!(directory.path().join("o.mp4").exists());
    // The converted name is found first; without -k the original goes.
    std::fs::remove_file(directory.path().join("o.mkv")).unwrap();
    let output = run(directory.path(), &remux, &url);
    assert!(stdout(&output).ends_with(
        "[VideoRemuxer] Remuxing video from mp4 to mkv; Destination: o.mkv\nDeleting original file o.mp4 (pass -k to keep)\n"
    ));
    assert!(!directory.path().join("o.mp4").exists());
    let output = run(directory.path(), &remux, &url);
    assert!(stdout(&output).ends_with(
        "[download] o.mkv has already been downloaded\n[VideoRemuxer] Not remuxing media file \"o.mkv\"; already is in target format mkv\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.mkv")).unwrap(),
        b"remuxed"
    );
    assert!(!std::fs::read_dir(directory.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".yt-dlp-rs-")
    }));
}

#[test]
fn without_continuing_or_part_files_an_existing_file_is_downloaded_again() {
    let url = format!("{}/v.mp4", serve());
    let directory = tempfile::tempdir().unwrap();
    let media = directory.path().join("o.mp4");
    let fresh = b"\0\0\0\x18ftypisom media bytes";
    std::fs::write(&media, b"old").unwrap();
    let output = run(directory.path(), &["--no-progress", "--no-continue"], &url);
    assert!(
        stdout(&output).ends_with("[download] Destination: o.mp4\n[download] Download completed\n")
    );
    assert_eq!(std::fs::read(&media).unwrap(), fresh);
    // --no-part continues the existing file itself; this server ignores
    // ranges, so the resume fails before the destination is named.
    std::fs::write(&media, b"old").unwrap();
    let output = run(directory.path(), &["--no-progress", "--no-part"], &url);
    assert!(stdout(&output).ends_with(
        "[download] Resuming download at byte 3\n[download] Unable to resume\n[download] Destination: o.mp4\n[download] Download completed\n"
    ));
    assert_eq!(std::fs::read(&media).unwrap(), fresh);
    // --no-overwrites still reuses it.
    std::fs::write(&media, b"old").unwrap();
    let output = run(
        directory.path(),
        &["--no-progress", "--no-continue", "-w"],
        &url,
    );
    assert!(stdout(&output).ends_with(
        "[download] o.mp4 has already been downloaded\n[download] Download completed\n"
    ));
    assert_eq!(std::fs::read(&media).unwrap(), b"old");
}
