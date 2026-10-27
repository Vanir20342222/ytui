//! Upstream `HttpFD` chunked downloading (`--http-chunk-size`, `--test`)
//! against a Range-capable local server; request patterns were compared with
//! pinned upstream.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

const BODY_LEN: usize = 25_000;

fn body() -> Vec<u8> {
    let mut body = b"\0\0\0\x18ftypisom".to_vec();
    body.extend((0..BODY_LEN - body.len()).map(|i| (i % 251) as u8));
    body
}

/// Serves `/v.mp4`, honoring `Range` unless `ranges` is false; records each
/// request's `Range` header.
fn serve(ranges: bool) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let requests = log.clone();
    thread::spawn(move || {
        let data = body();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let range = request
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .map(str::to_owned);
            requests.lock().unwrap().push(range.clone());
            let (status, slice, extra) = match range.filter(|_| ranges) {
                Some(range) => {
                    let (start, end) = range.split_once('-').unwrap();
                    let start: usize = start.parse().unwrap();
                    let end = end.parse().unwrap_or(data.len() - 1).min(data.len() - 1);
                    (
                        "206 Partial Content",
                        &data[start..=end],
                        format!("Content-Range: bytes {start}-{end}/{}\r\n", data.len()),
                    )
                }
                None => ("200 OK", &data[..], String::new()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                slice.len()
            );
            let _ = stream.write_all(slice);
        }
    });
    (origin, log)
}

fn run(origin: &str, directory: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .args(args)
        .arg(format!("{origin}/v.mp4"))
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn chunks_are_contiguous_ranges_of_95_to_100_percent() {
    let (origin, log) = serve(true);
    let directory = tempfile::tempdir().unwrap();
    let output = run(&origin, directory.path(), &["--http-chunk-size", "10K"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        body()
    );
    let log = log.lock().unwrap();
    // The extraction probe, then one request per chunk.
    assert_eq!(log[0], None);
    let mut next = 0;
    for range in &log[1..] {
        let (start, end) = range.as_deref().unwrap().split_once('-').unwrap();
        let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
        assert_eq!(start, next);
        let size = end - start + 1;
        assert!(
            end == BODY_LEN - 1 || (9728..=10240).contains(&size),
            "{range:?}"
        );
        next = end + 1;
    }
    assert_eq!(next, BODY_LEN);
    assert_eq!(log.len(), 4);
}

#[test]
fn test_mode_keeps_the_first_10241_bytes_even_without_range_support() {
    for ranges in [true, false] {
        let (origin, log) = serve(ranges);
        let directory = tempfile::tempdir().unwrap();
        let output = run(&origin, directory.path(), &["--test"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read(directory.path().join("o.mp4")).unwrap(),
            body()[..10241]
        );
        assert_eq!(log.lock().unwrap()[1..], [Some("0-10240".to_owned())]);
    }
    // A server ignoring Range sends the whole file in one response.
    let (origin, log) = serve(false);
    let directory = tempfile::tempdir().unwrap();
    let output = run(&origin, directory.path(), &["--http-chunk-size", "10K"]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        body()
    );
    assert_eq!(log.lock().unwrap().len(), 2);
    let output = run(&origin, directory.path(), &["--http-chunk-size", "ten"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("error: invalid http chunk size \"ten\" given")
    );
}

#[test]
fn progress_templates_render_info_and_progress_fields() {
    let (origin, _) = serve(true);
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--newline",
            "-o",
            "o.%(ext)s",
            "--progress-template",
        ])
        .arg("%(info.id)s|%(progress.status)s|%(progress._total_bytes_str)s|%(progress.filename)s")
        .arg(format!("{origin}/v.mp4"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Upstream's `report_progress` dict: right-aligned byte strings.
    assert!(
        stdout.contains("\nv|downloading|  24.41KiB|o.mp4\n"),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("\nv|finished|  24.41KiB|o.mp4\n"),
        "{stdout}"
    );
}
