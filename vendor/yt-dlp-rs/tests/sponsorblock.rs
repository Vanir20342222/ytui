//! SponsorBlock (`SponsorBlockPP` with `ModifyChaptersPP`) against a local
//! API stand-in; lines and chapters were checked against pinned upstream
//! with the same saved response and real media.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

const SEGMENTS: &str = r#"[{"videoID":"dQw4w9WgXcQ","segments":[{"segment":[0.5,2.0],"category":"intro","actionType":"skip","videoDuration":10.0,"description":""},{"segment":[4.0,6.0],"category":"sponsor","actionType":"skip","videoDuration":10.0,"description":""},{"segment":[7.0,7.0],"category":"poi_highlight","actionType":"poi","videoDuration":10.0,"description":""},{"segment":[8.0,9.5],"category":"selfpromo","actionType":"skip","videoDuration":30.0,"description":""}]},{"videoID":"other","segments":[]}]"#;

/// Serves the media and the saved `skipSegments` response, recording paths.
fn serve() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let record = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            record.lock().unwrap().push(path.clone());
            let (status, mime, body): (&str, &str, &[u8]) =
                if path.starts_with("/api/skipSegments/5f6b?") {
                    ("200 OK", "application/json", SEGMENTS.as_bytes())
                } else if path.starts_with("/api/") {
                    ("404 Not Found", "text/plain", b"Not Found")
                } else {
                    ("200 OK", "video/mp4", b"\0\0\0\x18ftypisom media bytes")
                };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    (origin, requests)
}

fn tools(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let ffmpeg = "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
        dir=\"${0%/ffmpeg}\"\nprev=\nfor a in \"$@\"; do\n  case \"$prev:$a\" in -i:*.concat) cp \"$a\" \"$dir/spec.txt\";; esac\n  \
        prev=\"$a\"; last=\"$a\"\ndone\nprintf 'cut' > \"$last\"\n";
    let ffprobe = "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffprobe version test'; exit 0; fi\n\
        printf '{\"format\": {\"duration\": \"10.000000\"}}'\n";
    for (name, script) in [("ffmpeg", ffmpeg), ("ffprobe", ffprobe)] {
        let path = directory.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run(directory: &Path, tools: &Path, origin: &str, id: &str, args: &[&str]) -> Output {
    let info = serde_json::json!({
        "id": id, "title": "Vid", "ext": "mp4", "duration": 10,
        "url": format!("{origin}/v.mp4"),
        "webpage_url": format!("https://www.youtube.com/watch?v={id}"),
        "extractor": "youtube", "extractor_key": "Youtube",
        "formats": [{"format_id": "18", "url": format!("{origin}/v.mp4"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
        "chapters": [{"start_time": 0, "end_time": 5.0, "title": "Part A"},
            {"start_time": 5.0, "end_time": 10, "title": "Part B"}],
    });
    let path = directory.join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .arg("--ffmpeg-location")
        .arg(tools)
        .arg("--load-info-json")
        .arg(&path)
        .args(["--sponsorblock-api", origin])
        .args(args)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn success(output: &Output) -> (String, String) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const MISMATCH: &str = "WARNING: Some SponsorBlock segments are from a video of different duration, maybe from an old version of this video\n";

#[test]
fn marked_segments_become_chapters_with_the_title_template() {
    let (origin, requests) = serve();
    let tools_dir = tempfile::tempdir().unwrap();
    tools(tools_dir.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        "dQw4w9WgXcQ",
        &[
            "-s",
            "--sponsorblock-mark",
            "sponsor,intro",
            "--print",
            "%(sponsorblock_chapters.:.title)s",
        ],
    );
    let (stdout, stderr) = success(&output);
    // The stand-in ignores the requested categories; the API filters them.
    assert_eq!(
        stdout,
        "['Intermission/Intro Animation', 'Sponsor', 'Highlight']\n"
    );
    assert_eq!(stderr, MISMATCH);
    let query = requests.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        query,
        "/api/skipSegments/5f6b?service=YouTube&categories=%5B%22sponsor%22%2C+%22intro%22%5D&actionTypes=%5B%22skip%22%2C+%22poi%22%2C+%22chapter%22%5D"
    );

    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        "dQw4w9WgXcQ",
        &[
            "--sponsorblock-mark",
            "all",
            "--sponsorblock-chapter-title",
            "SB %(category)s",
            "--print",
            "after_move:%(chapters.:.title)s",
        ],
    );
    let (stdout, _) = success(&output);
    assert_eq!(
        stdout,
        // Identically named neighbouring sponsor chapters merge.
        "['SB intro', 'Part A', 'SB sponsor', 'Part B', 'SB poi_highlight', 'Part B']\n"
    );
}

#[test]
fn removed_segments_are_cut_and_missing_data_is_reported() {
    let (origin, _) = serve();
    let tools_dir = tempfile::tempdir().unwrap();
    tools(tools_dir.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        "dQw4w9WgXcQ",
        &["--sponsorblock-remove", "sponsor"],
    );
    let (stdout, _) = success(&output);
    assert!(stdout.starts_with(
        "[SponsorBlock] Fetching SponsorBlock segments\n[SponsorBlock] Found 3 segments in the SponsorBlock database\n"
    ));
    assert!(stdout.ends_with(
        "[ModifyChapters] Removing chapters from o.mp4\nDeleting original file o.uncut.mp4 (pass -k to keep)\n"
    ));
    let spec = std::fs::read_to_string(tools_dir.path().join("spec.txt")).unwrap();
    assert!(spec.contains("\noutpoint 4.000000\n") && spec.contains("\ninpoint 6.000000\n"));

    // A 404 means no segments.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        "unknownvid1",
        &["--sponsorblock-remove", "sponsor"],
    );
    let (stdout, _) = success(&output);
    assert!(
        stdout.contains(
            "[SponsorBlock] No matching segments were found in the SponsorBlock database\n"
        )
    );
    assert!(stdout.ends_with("[ModifyChapters] SponsorBlock information is unavailable\n"));
}

#[test]
fn unreachable_api_is_a_deferred_preprocessing_error() {
    let tools_dir = tempfile::tempdir().unwrap();
    tools(tools_dir.path());
    let (origin, _) = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &origin,
        "dQw4w9WgXcQ",
        &[
            "-s",
            "--sponsorblock-api",
            "http://127.0.0.1:9",
            "--extractor-retries",
            "1",
            "--sponsorblock-mark",
            "sponsor",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{stderr}");
    assert!(lines[0].starts_with("WARNING: Unable to communicate with SponsorBlock API: [Errno "));
    assert!(lines[0].ends_with(". Retrying (1/1)..."));
    assert!(
        lines[1].starts_with("ERROR: Preprocessing: Unable to communicate with SponsorBlock API: ")
    );
    assert_eq!(lines[1], lines[2]);
}

#[test]
fn categories_follow_upstream_validation() {
    let directory = tempfile::tempdir().unwrap();
    for (args, message) in [
        (
            vec!["--sponsorblock-mark", "bogus"],
            "wrong CATS for --sponsorblock-mark: bogus",
        ),
        (
            vec!["--sponsorblock-remove", "chapter"],
            "wrong CATS for --sponsorblock-remove: chapter",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .arg("--ignore-config")
            .args(&args)
            .arg("http://127.0.0.1:9/v.mp4")
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).ends_with(&format!("error: {message}\n")));
    }
}
