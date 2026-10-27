//! Upstream `process_info`/`HttpFD` error reporting for media and subtitle
//! downloads, checked against pinned upstream with the same saved info JSON.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

/// A media playlist whose second segment is missing.
const GAP: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\ns1.ts\n#EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n";

/// Serves `/v.mp4` and the `/gap.m3u8` playlist; `/busy*` answers 503 and anything else 404 with a
/// non-canonical reason phrase like Python's `http.server`.
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
            let (status, body): (&str, &[u8]) = match path.as_str() {
                "/v.mp4" => ("200 OK", b"media"),
                "/gap.m3u8" => ("200 OK", GAP.as_bytes()),
                "/s0.ts" | "/s2.ts" => ("200 OK", b"segment"),
                path if path.starts_with("/busy") => ("503 Service Unavailable", b"busy"),
                _ => ("404 File not found", b"missing"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

fn run(directory: &Path, media: &str, subtitle: Option<&str>, args: &[&str]) -> Output {
    let origin = serve();
    let mut info = serde_json::json!({
        "id": "vid", "title": "Vid", "ext": "mp4",
        "webpage_url": format!("{origin}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": [{"format_id": "0", "url": format!("{origin}{media}"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
    });
    if let Some(subtitle) = subtitle {
        info["subtitles"] =
            serde_json::json!({"en": [{"url": format!("{origin}{subtitle}"), "ext": "vtt"}]});
    }
    let path = directory.join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .arg("--load-info-json")
        .arg(&path)
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn media_http_errors_are_reported_before_any_destination() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), "/missing.mp4", None, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stdout),
        "[info] vid: Downloading 1 format(s): 0\n"
    );
    assert_eq!(
        text(&output.stderr),
        "ERROR: unable to download video data: HTTP Error 404: File not found\n"
    );
    assert!(!directory.path().join("o.mp4").exists());
}

#[test]
fn retryable_errors_report_through_the_retry_manager() {
    for (retries, stdout, error) in [
        (
            "0",
            "",
            "ERROR: \r[download] Got error: HTTP Error 503: Service Unavailable\n",
        ),
        (
            "1",
            "[download] Got error: HTTP Error 503: Service Unavailable. Retrying (1/1)...\n",
            "ERROR: \r[download] Got error: HTTP Error 503: Service Unavailable. Giving up after 1 retries\n",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = run(
            directory.path(),
            "/busy.mp4",
            None,
            &["-R", retries, "--retry-sleep", "0"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            text(&output.stdout),
            format!("[info] vid: Downloading 1 format(s): 0\n{stdout}")
        );
        assert_eq!(text(&output.stderr), error);
    }
}

#[test]
fn subtitle_failures_warn_under_ignore_errors() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        "/v.mp4",
        Some("/nosub.vtt"),
        &["--write-subs", "-i"],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    // No destination is named for the failed subtitle.
    assert_eq!(
        text(&output.stdout),
        "[info] vid: Downloading subtitles: en\n[info] vid: Downloading 1 format(s): 0\n\
         [info] Writing video subtitles to: o.en.vtt\n[download] Destination: o.mp4\n\
         [download] Download completed\n"
    );
    assert_eq!(
        text(&output.stderr),
        "WARNING: Unable to download video subtitles for 'en': HTTP Error 404: File not found\n"
    );
    assert!(directory.path().join("o.mp4").is_file());
}

#[test]
fn a_raised_subtitle_failure_retries_the_loaded_info_with_its_url() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        "/v.mp4",
        Some("/nosub.vtt"),
        &["--write-subs"],
    );
    // The retry succeeds, so the run does too.
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.starts_with(
            "[info] vid: Downloading subtitles: en\n[info] vid: Downloading 1 format(s): 0\n\
             [info] Writing video subtitles to: o.en.vtt\n[generic] Extracting URL: http://"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("[info] There are no subtitles for the requested languages\n"));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with(
            "\r\nWARNING: The info failed to download: Unable to download video subtitles for 'en': HTTP Error 404: File not found; trying with URL http://"
        ),
        "{stderr}"
    );
    assert!(directory.path().join("o.mp4").is_file());
}

#[test]
fn transport_errors_use_urllib_errno_text() {
    // A port that was just bound and released refuses connections.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let directory = tempfile::tempdir().unwrap();
    let info = serde_json::json!({
        "id": "vid", "title": "Vid", "ext": "mp4",
        "webpage_url": format!("http://{address}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": [{"format_id": "0", "url": format!("http://{address}/v.mp4"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
    });
    let path = directory.path().join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-progress",
            "-R",
            "1",
            "--retry-sleep",
            "0",
        ])
        .arg("--load-info-json")
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = text(&output.stdout);
    let retry = stdout.lines().nth(1).unwrap();
    let (prefix, refused) = if cfg!(windows) {
        (
            "[download] Got error: [WinError ",
            "] No connection could be made because the target machine actively refused it. Retrying (1/1)...",
        )
    } else {
        (
            "[download] Got error: [Errno ",
            "] Connection refused. Retrying (1/1)...",
        )
    };
    assert!(
        retry.starts_with(prefix) && retry.ends_with(refused),
        "{stdout}"
    );
    let gave_up = if cfg!(windows) {
        "] No connection could be made because the target machine actively refused it. Giving up after 1 retries\n"
    } else {
        "] Connection refused. Giving up after 1 retries\n"
    };
    assert!(
        text(&output.stderr).ends_with(gave_up),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn unavailable_fragments_are_skipped_or_end_the_download() {
    let origin = serve();
    for abort in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command
            .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
            .args(["--fragment-retries", "1", "--retry-sleep", "fragment:0"])
            .current_dir(directory.path());
        if abort {
            command.arg("--abort-on-unavailable-fragments");
        }
        let output = command.arg(format!("{origin}/gap.m3u8")).output().unwrap();
        let stdout = text(&output.stdout);
        assert!(stdout.contains(
            "[download] Got error: HTTP Error 404: File not found. Retrying fragment 2 (1/1)...\n"
        ));
        if abort {
            assert_eq!(output.status.code(), Some(1));
            assert!(text(&output.stderr).ends_with(
                "ERROR: \r[download] Got error: HTTP Error 404: File not found. Giving up after 1 retries\n\
                 ERROR: fragment 2 not found, unable to continue\n"
            ));
        } else {
            assert!(output.status.success(), "{}", text(&output.stderr));
            assert!(stdout.contains("[download] fragment not found; Skipping fragment 2 ...\n"));
            assert_eq!(
                std::fs::read(directory.path().join("o.mp4")).unwrap(),
                b"segmentsegment"
            );
        }
    }
}

/// Upstream `FragmentFD` resume: an aborted download keeps `FILE.part` and
/// `FILE.ytdl`, and `--continue` skips the fragments already written.
#[test]
fn interrupted_fragment_downloads_resume_from_the_ytdl_state() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let available = Arc::new(AtomicBool::new(false));
    let first_requests = Arc::new(AtomicUsize::new(0));
    let (serve_available, serve_first) = (available.clone(), first_requests.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body): (&str, &[u8]) = match path.as_str() {
                "/gap.m3u8" => ("200 OK", GAP.as_bytes()),
                "/s0.ts" => {
                    serve_first.fetch_add(1, Ordering::Relaxed);
                    ("200 OK", b"first-")
                }
                "/s1.ts" if serve_available.load(Ordering::Relaxed) => ("200 OK", b"second-"),
                "/s2.ts" => ("200 OK", b"third"),
                _ => ("404 Not Found", b"missing"),
            };
            let mime = if path.ends_with(".m3u8") {
                "application/vnd.apple.mpegurl"
            } else {
                "video/mp2t"
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
            .args([
                "--fragment-retries",
                "0",
                "--abort-on-unavailable-fragments",
            ])
            .arg(format!("{origin}/gap.m3u8"))
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    let output = run();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4.part")).unwrap(),
        b"first-"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("o.mp4.ytdl")).unwrap(),
        r#"{"downloader": {"current_fragment": {"index": 1}, "extra_state": {}}}"#
    );
    let fetched = first_requests.load(Ordering::Relaxed);
    available.store(true, Ordering::Relaxed);
    let output = run();
    assert!(
        output.status.success(),
        "{}\n{}",
        text(&output.stdout),
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"first-second-third"
    );
    // The first fragment was not fetched again; the state is removed.
    assert_eq!(first_requests.load(Ordering::Relaxed), fetched);
    assert!(!directory.path().join("o.mp4.ytdl").exists());
    assert!(!directory.path().join("o.mp4.part").exists());
}
