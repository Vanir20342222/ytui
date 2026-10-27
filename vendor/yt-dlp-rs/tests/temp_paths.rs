//! `-P temp:` and `MoveFilesAfterDownloadPP`: files are written beside the
//! temporary media name and moved to their final names afterwards. Lines were
//! checked against pinned upstream with the same saved info JSON.
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
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (mime, body): (&str, &[u8]) = match path.as_str() {
                "/s.vtt" => ("text/vtt", b"WEBVTT\n\n00:00.000 --> 00:01.000\nHi\n"),
                "/t.jpg" => ("image/jpeg", b"\xff\xd8\xff\xe0 thumbnail"),
                _ => ("video/mp4", b"\0\0\0\x18ftypisom media bytes"),
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

/// Records nothing; writes its last argument like a successful FFmpeg.
fn fake_ffmpeg(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
         for a in \"$@\"; do last=\"$a\"; done\nprintf 'merged' > \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(directory: &Path, origin: &str, merged: bool, args: &[&str]) -> Output {
    let formats = if merged {
        serde_json::json!([
            {"format_id": "v", "url": format!("{origin}/v.mp4"), "ext": "mp4",
             "protocol": "http", "vcodec": "avc1", "acodec": "none"},
            {"format_id": "a", "url": format!("{origin}/v.mp4"), "ext": "m4a",
             "protocol": "http", "vcodec": "none", "acodec": "mp4a.40.2"},
        ])
    } else {
        serde_json::json!([{"format_id": "0", "url": format!("{origin}/v.mp4"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}])
    };
    let info = serde_json::json!({
        "id": "vid", "title": "Vid", "ext": "mp4",
        "webpage_url": format!("{origin}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": formats,
        "thumbnails": [{"id": "0", "url": format!("{origin}/t.jpg")}],
        "subtitles": {"en": [{"url": format!("{origin}/s.vtt"), "ext": "vtt"}]},
    });
    let path = directory.join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    let tools = directory.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    fake_ffmpeg(&tools);
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .arg("--ffmpeg-location")
        .arg(&tools)
        .arg("--load-info-json")
        .arg(&path)
        .args(args)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn success(output: &Output, directory: &Path) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = std::fs::canonicalize(directory).unwrap();
    String::from_utf8_lossy(&output.stdout)
        .replace(&*root.to_string_lossy(), "D")
        .replace(&*directory.to_string_lossy(), "D")
}

fn lines(stdout: &str, prefix: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| line.starts_with(prefix))
        .map(str::to_owned)
        .collect()
}

#[test]
fn media_and_sidecars_move_out_of_the_temporary_directory() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    let output = run(
        dir,
        &origin,
        false,
        &[
            "-P",
            "temp:t",
            "-P",
            "out",
            "-P",
            "thumbnail:th",
            "--write-subs",
            "--write-thumbnail",
            "--write-info-json",
        ],
    );
    let stdout = success(&output, dir);
    assert!(stdout.contains("[info] Writing video subtitles to: out/t/o.en.vtt\n"));
    assert!(stdout.contains("[info] Writing video thumbnail 0 to: out/t/o.jpg\n"));
    // Info JSON goes straight to its final name.
    assert!(stdout.contains("[info] Writing video metadata as JSON to: out/o.info.json\n"));
    assert!(stdout.contains("[download] Destination: out/t/o.mp4\n"));
    assert_eq!(
        lines(&stdout, "[MoveFiles]"),
        [
            "[MoveFiles] Moving file \"out/t/o.en.vtt\" to \"out/o.en.vtt\"",
            "[MoveFiles] Moving file \"out/t/o.jpg\" to \"out/th/o.jpg\"",
            "[MoveFiles] Moving file \"out/t/o.mp4\" to \"D/out/o.mp4\"",
        ]
    );
    for file in ["o.mp4", "o.en.vtt", "th/o.jpg", "o.info.json"] {
        assert!(dir.join("out").join(file).is_file(), "{file}");
    }
    assert_eq!(std::fs::read_dir(dir.join("out/t")).unwrap().count(), 0);

    // Without `temp`, typed paths still move the sidecars.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &origin,
        false,
        &["--write-subs", "-o", "subtitle:subs/s.%(ext)s"],
    );
    let stdout = success(&output, directory.path());
    assert!(stdout.contains("[info] Writing video subtitles to: o.en.vtt\n"));
    assert_eq!(
        lines(&stdout, "[MoveFiles]"),
        ["[MoveFiles] Moving file \"o.en.vtt\" to \"subs/s.en.vtt\""]
    );
}

#[test]
fn stage_paths_follow_the_move() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &origin,
        false,
        &[
            "-P",
            "temp:t",
            "--print",
            "before_dl:filename",
            "--print",
            "post_process:filepath",
            "--print",
            "after_move:filepath",
        ],
    );
    assert_eq!(
        success(&output, directory.path()),
        "o.mp4\nt/o.mp4\nD/o.mp4\n"
    );
    // --skip-download moves only the sidecars.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &origin,
        false,
        &["-P", "temp:t", "--write-subs", "--skip-download"],
    );
    let stdout = success(&output, directory.path());
    assert!(stdout.ends_with("[MoveFiles] Moving file \"t/o.en.vtt\" to \"o.en.vtt\"\n"));
    assert!(directory.path().join("o.en.vtt").is_file());
}

#[test]
fn kept_formats_move_and_existing_targets_follow_overwrites() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    let output = run(dir, &origin, true, &["-P", "temp:t", "-k"]);
    let stdout = success(&output, dir);
    assert!(stdout.contains("[Merger] Merging formats into \"t/o.mp4\"\n"));
    assert_eq!(
        lines(&stdout, "[MoveFiles]"),
        [
            "[MoveFiles] Moving file \"t/o.fv.mp4\" to \"D/o.fv.mp4\"",
            "[MoveFiles] Moving file \"t/o.fa.m4a\" to \"D/o.fa.m4a\"",
            "[MoveFiles] Moving file \"t/o.mp4\" to \"D/o.mp4\"",
        ]
    );
    assert_eq!(std::fs::read(dir.join("o.mp4")).unwrap(), b"merged");

    // An existing target is replaced unless --no-overwrites.
    for (overwrite, warning, kept) in [
        (
            None,
            "WARNING: Replacing existing file \"D/o.fv.mp4\"\n",
            false,
        ),
        (
            Some("--no-overwrites"),
            "WARNING: Cannot move file \"t/o.fv.mp4\" out of temporary directory since \"D/o.fv.mp4\" already exists. \n",
            true,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        std::fs::write(dir.join("o.fv.mp4"), "seed").unwrap();
        let mut args = vec!["-P", "temp:t", "-k"];
        args.extend(overwrite);
        let output = run(dir, &origin, true, &args);
        success(&output, dir);
        let root = std::fs::canonicalize(dir).unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr)
            .replace(&*root.to_string_lossy(), "D")
            .replace(&*dir.to_string_lossy(), "D");
        assert_eq!(stderr, warning);
        assert_eq!(dir.join("t/o.fv.mp4").exists(), kept);
        assert_eq!(
            std::fs::read(dir.join("o.fv.mp4")).unwrap() == b"seed",
            kept
        );
    }
}
