//! `--remove-chapters` (upstream `ModifyChaptersPP`) with recording FFmpeg
//! and ffprobe stand-ins; lines and chapter results were checked against
//! pinned upstream with real media.
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

/// `ffmpeg` copies a concat spec input to `spec.txt` and writes `cut` to its
/// output; `ffprobe` reports a 10 second duration.
fn tools(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let ffmpeg = "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
        dir=\"${0%/ffmpeg}\"\nprev=\nfor a in \"$@\"; do\n  case \"$prev:$a\" in -i:*.concat) cp \"$a\" \"$dir/spec.txt\";; esac\n  \
        prev=\"$a\"; last=\"$a\"\ndone\nprintf '%s\\n' \"$@\" >> \"$dir/args.txt\"\nprintf 'cut' > \"$last\"\n";
    let ffprobe = "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffprobe version test'; exit 0; fi\n\
        printf '{\"format\": {\"duration\": \"10.000000\"}}'\n";
    for (name, script) in [("ffmpeg", ffmpeg), ("ffprobe", ffprobe)] {
        let path = directory.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run(directory: &Path, tools: &Path, origin: &str, args: &[&str]) -> Output {
    let info = serde_json::json!({
        "id": "chap", "title": "Chap", "ext": "mp4", "duration": 10,
        "url": format!("{origin}/v.mp4"), "webpage_url": format!("{origin}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": [{"format_id": "0", "url": format!("{origin}/v.mp4"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
        "chapters": [{"start_time": 2.0, "title": "Alpha"}, {"start_time": 4.5, "title": "Beta"}],
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

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn matching_chapters_are_cut_and_the_info_dict_updated() {
    let origin = serve();
    let tools_dir = tempfile::tempdir().unwrap();
    tools(tools_dir.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        &[
            "--remove-chapters",
            "Alpha",
            "--print",
            "after_move:%(chapters.:.title)s %(chapters.-1.end_time)s",
        ],
    );
    let stdout = success(&output);
    assert_eq!(stdout, "['<Untitled Chapter 1>', 'Beta'] 7.5\n");
    let spec = std::fs::read_to_string(tools_dir.path().join("spec.txt")).unwrap();
    let lines: Vec<&str> = spec.lines().collect();
    assert_eq!(lines[0], "ffconcat version 1.0");
    assert!(lines[1].starts_with("file 'file:") && lines[1].ends_with("source.mp4'"));
    assert_eq!(lines[2], "outpoint 2.000000");
    assert!(lines[3].starts_with("file 'file:"));
    assert_eq!(lines[4], "inpoint 4.500000");
    assert_eq!(lines.len(), 5);
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"cut"
    );
    assert!(!directory.path().join("o.uncut.mp4").exists());

    // The screen lines; -k keeps the uncut original.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        &["--remove-chapters", "^B", "-k", "--force-keyframes-at-cuts"],
    );
    let stdout = success(&output);
    assert!(stdout.ends_with(
        "[download] Download completed\n[ModifyChapters] Re-encoding \"o.mp4\" with appropriate keyframes\n[ModifyChapters] Removing chapters from o.mp4\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.uncut.mp4")).unwrap(),
        b"\0\0\0\x18ftypisom media bytes"
    );
    let args = std::fs::read_to_string(tools_dir.path().join("args.txt")).unwrap();
    assert!(args.contains("-force_key_frames\n4.500000,10.000000\n"));
}

#[test]
fn unmatched_or_whole_removals_keep_the_file() {
    let origin = serve();
    let tools_dir = tempfile::tempdir().unwrap();
    tools(tools_dir.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        &["--remove-chapters", "Gamma"],
    );
    assert!(
        success(&output).ends_with("[ModifyChapters] There are no chapters matching the regex\n")
    );
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        &["--remove-chapters", "."],
    );
    success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "WARNING: You have requested to remove the entire video, which is not possible\n"
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"\0\0\0\x18ftypisom media bytes"
    );
}

#[test]
fn removal_options_follow_parse_chapters() {
    let directory = tempfile::tempdir().unwrap();
    let usage = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .arg("--ignore-config")
            .args(args)
            .arg("http://127.0.0.1:9/v.mp4")
            .current_dir(directory.path())
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (code, stderr) = usage(&["--remove-chapters", "*-1-2"]);
    assert_eq!(code, Some(2));
    assert!(stderr.ends_with(
        "error: invalid --remove-chapters time range \"*-1-2\". Negative timestamps are not allowed\n"
    ));
    let (code, stderr) = usage(&["--remove-chapters", "*from-url"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("Must be of the form \"*start-end\""));
    let (code, _) = usage(&["--remove-chapters", "x", "--remove-chapters", "*1-2"]);
    assert_eq!(code, Some(1));
}
