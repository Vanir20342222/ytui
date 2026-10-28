//! `--max-downloads` and `--min/max-filesize` through local HTTP media and the CLI.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, body) = if path == "/big.mp4" {
                    (200, "x".repeat(96 * 1024))
                } else if path.ends_with(".mp4") {
                    (200, "media".to_owned())
                } else {
                    (404, "missing".to_owned())
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            origin,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cli(args: &[&str], directory: &Path, stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin.unwrap_or_default()).unwrap();
    drop(input);
    child.wait_with_output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn max_downloads_stops_after_processed_videos_with_exit_101() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let (a, b) = (server.url("/a.mp4"), server.url("/b.mp4"));
    let output = cli(&["--max-downloads", "1", &a, &b], directory.path(), None);
    assert_eq!(output.status.code(), Some(101), "{}", stderr(&output));
    assert!(
        stdout(&output).contains(
            "[info] Maximum number of downloads reached, stopping due to --max-downloads"
        )
    );
    assert!(directory.path().join("a [a].mp4").exists());
    assert!(!directory.path().join("b [b].mp4").exists());
    // Simulated videos count too; 0 means no limit.
    let output = cli(
        &["--max-downloads", "1", "-O", "id", &a, &b],
        directory.path(),
        None,
    );
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\n");
    let output = cli(
        &["--max-downloads", "0", "-O", "id", &a, &b],
        directory.path(),
        None,
    );
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\nb\n");
}

#[test]
fn filesize_limits_skip_media_without_error_or_archive() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    let url = server.url("/a.mp4");
    for (flag, size, message) in [
        (
            "--max-filesize",
            "4",
            "File is larger than max-filesize (5 bytes > 4 bytes). Aborting.",
        ),
        (
            "--min-filesize",
            "1k",
            "File is smaller than min-filesize (5 bytes < 1024 bytes). Aborting.",
        ),
    ] {
        let output = cli(
            &[
                flag,
                size,
                "--download-archive",
                archive.to_str().unwrap(),
                &url,
            ],
            directory.path(),
            None,
        );
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            stdout(&output).contains(&format!("[download] {message}")),
            "{}",
            stdout(&output)
        );
        assert!(!archive.exists());
        assert!(!directory.path().join("a [a].mp4").exists());
    }
    let output = cli(&["--max-filesize", "5", &url], directory.path(), None);
    assert!(output.status.success());
    assert!(directory.path().join("a [a].mp4").exists());
    let invalid = cli(&["--min-filesize", "1KiB", &url], directory.path(), None);
    assert!(!invalid.status.success());
    assert!(stderr(&invalid).contains("invalid min filesize \"1KiB\" given"));
}

#[test]
fn quiet_and_no_warnings_suppress_screen_and_warning_lines() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/a.mp4");
    let output = cli(&["-q", &url], directory.path(), None);
    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    assert!(directory.path().join("a [a].mp4").exists());
    // Errors are still reported when quiet.
    let output = cli(&["-q", &server.url("/missing.txt")], directory.path(), None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("ERROR:"));
    // --no-quiet restores messages; the last flag wins.
    let output = cli(
        &["-q", "--no-quiet", "--force-overwrites", &url],
        directory.path(),
        None,
    );
    assert!(stdout(&output).contains("[download] Destination:"));
    let output = cli(
        &[
            "--no-warnings",
            "--remux-video",
            "mp4",
            "--recode-video",
            "mkv",
            "-s",
            &url,
        ],
        directory.path(),
        None,
    );
    assert!(!stderr(&output).contains("WARNING:"), "{}", stderr(&output));
    let output = cli(
        &["--remux-video", "mp4", "--recode-video", "mkv", "-s", &url],
        directory.path(),
        None,
    );
    assert!(stderr(&output).contains("WARNING: --remux-video is ignored"));
}

#[test]
fn limit_rate_slows_media_transfers_and_rejects_zero() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/big.mp4");
    let start = std::time::Instant::now();
    let output = cli(&["-r", "128K", &url], directory.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    // 96 KiB at 128 KiB/s takes at least ~0.75 s.
    assert!(
        start.elapsed() >= Duration::from_millis(600),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        std::fs::metadata(directory.path().join("big [big].mp4"))
            .unwrap()
            .len(),
        96 * 1024
    );
    let zero = cli(&["--limit-rate", "0", &url], directory.path(), None);
    assert!(!zero.status.success());
    assert!(stderr(&zero).contains("rate limit \"0\" must be positive"));
}

#[test]
fn sleep_interval_pauses_before_media_and_validates_like_upstream() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/a.mp4");
    let start = std::time::Instant::now();
    let output = cli(
        &[
            "--min-sleep-interval",
            "0.4",
            "--max-sleep-interval",
            "0.5",
            &url,
        ],
        directory.path(),
        None,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(start.elapsed() >= Duration::from_millis(400));
    let slept: f64 = stdout(&output)
        .split("[download] Sleeping ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|seconds| seconds.parse().ok())
        .unwrap();
    assert!((0.4..=0.5).contains(&slept), "{slept}");
    // Already-downloaded files do not sleep.
    let output = cli(&["--sleep-interval", "5", &url], directory.path(), None);
    assert!(output.status.success());
    assert!(!stdout(&output).contains("Sleeping"));
    for (args, message) in [
        (
            vec!["--max-sleep-interval", "1"],
            "min sleep interval must be specified",
        ),
        (
            vec!["--sleep-interval", "2", "--max-sleep-interval", "1"],
            "max sleep interval \"1.0\" must be must be greater than or equal to min sleep interval \"2.0\"",
        ),
        (
            vec!["--sleep-subtitles", "-1"],
            "subtitles sleep interval \"-1.0\" must be positive or 0",
        ),
    ] {
        let mut all = args.clone();
        all.push(&url);
        let output = cli(&all, directory.path(), None);
        assert!(!output.status.success());
        assert!(stderr(&output).contains(message), "{}", stderr(&output));
    }
}

#[test]
fn match_filters_skip_or_break_like_upstream() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        server.url("/a.mp4"),
        server.url("/b.mp4"),
        server.url("/c.mp4"),
    );
    let ids = |output: &Output| String::from_utf8_lossy(&output.stdout).into_owned();
    let output = cli(
        &["--match-filters", "title=a", "-O", "id", &a, &b],
        directory.path(),
        None,
    );
    assert!(output.status.success());
    assert_eq!(ids(&output), "a\n");
    // Printing is quiet; the skip reason is a screen message.
    let output = cli(
        &["--match-filters", "title=a", "-s", &b],
        directory.path(),
        None,
    );
    assert!(stdout(&output).contains("[download] b does not pass filter (title=a), skipping .."));
    // Repeated filters are alternatives; --no-match-filters discards earlier ones.
    let output = cli(
        &[
            "--match-filters",
            "title=a",
            "--match-filters",
            "id=c",
            "-O",
            "id",
            &a,
            &b,
            &c,
        ],
        directory.path(),
        None,
    );
    assert_eq!(ids(&output), "a\nc\n");
    let output = cli(
        &[
            "--match-filters",
            "title=a",
            "--no-match-filters",
            "-O",
            "id",
            &a,
            &b,
        ],
        directory.path(),
        None,
    );
    assert_eq!(ids(&output), "a\nb\n");
    // Format fields pass before selection and are checked once complete.
    // Checked against pinned upstream: the direct format has no filesize, so
    // the complete check skips it unless the comparison allows unknown values.
    let output = cli(
        &["--match-filters", "filesize>4 & !height", "-O", "id", &a],
        directory.path(),
        None,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(ids(&output), "", "{}", stderr(&output));
    let output = cli(
        &["--match-filters", "filesize>?4 & !height", "-O", "id", &a],
        directory.path(),
        None,
    );
    assert_eq!(ids(&output), "a\n", "{}", stderr(&output));
    let output = cli(
        &["--break-match-filters", "id!=b", &a, &b, &c],
        directory.path(),
        None,
    );
    assert_eq!(output.status.code(), Some(101), "{}", stderr(&output));
    assert!(stdout(&output).contains(
        "[download] b does not pass filter (id!=b), skipping ..\n[info] Encountered a video that did not match filter, stopping due to --break-match-filter\nAborting remaining downloads"
    ));
    assert!(directory.path().join("a [a].mp4").exists());
    assert!(!directory.path().join("c [c].mp4").exists());
    let invalid = cli(&["--match-filters", "x ?? 5", &a], directory.path(), None);
    assert!(!invalid.status.success());
    assert!(stderr(&invalid).contains("Invalid filter part"));
}

#[test]
fn write_description_reports_missing_descriptions_without_files() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/a.mp4");
    let output = cli(
        &["--write-description", "--skip-download", &url],
        directory.path(),
        None,
    );
    assert!(output.status.success());
    assert!(stdout(&output).contains("[info] There's no video description to write"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    // Simulation never writes descriptions; the last flag wins.
    let output = cli(
        &["--write-description", "--no-write-description", &url],
        directory.path(),
        None,
    );
    assert!(!stdout(&output).contains("description"));
}
