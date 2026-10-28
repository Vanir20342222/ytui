//! `-a`/`--batch-file` URL lists through local HTTP extraction and the CLI.
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
                let (status, body) = if path.ends_with(".mp4") {
                    (200, "media")
                } else {
                    (404, "missing")
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

fn printed_ids(output: &Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn batch_urls_precede_positional_urls_and_skip_comments() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let batch = format!(
        "\u{feff}  {a}  # first\r\n# {skip}\n; {skip}\n] {skip}\n\n{b}\t\n",
        a = server.url("/a.mp4"),
        b = server.url("/b.mp4#frag"),
        skip = server.url("/skip.mp4"),
    );
    std::fs::write(directory.path().join("urls.txt"), batch).unwrap();
    let positional = format!("  {}  ", server.url("/c.mp4"));
    let output = cli(
        &["-a", "urls.txt", "--print", "id", &positional],
        directory.path(),
        None,
    );
    assert_eq!(printed_ids(&output), ["a", "b", "c"]);
}

#[test]
fn stdin_batch_file_announces_reading_and_allows_no_positional_urls() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let input = format!("{}\n", server.url("/stdin.mp4"));
    let output = cli(
        &["--batch-file", "-", "-O", "id"],
        directory.path(),
        Some(input.as_bytes()),
    );
    assert_eq!(printed_ids(&output), ["stdin"]);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Reading URLs from STDIN - EOF (Ctrl+D) to end:")
    );
}

#[test]
fn unreadable_empty_and_disabled_batch_files_fail_before_requests() {
    let directory = tempfile::tempdir().unwrap();
    let missing = cli(&["-a", "missing.txt", "-O", "id"], directory.path(), None);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr)
            .contains("ERROR: batch file missing.txt could not be read")
    );
    std::fs::write(directory.path().join("empty.txt"), "# only comments\n").unwrap();
    let empty = cli(&["-a", "empty.txt"], directory.path(), None);
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("You must provide at least one URL."));
    // The last of --batch-file/--no-batch-file wins.
    let disabled = cli(
        &["-a", "missing.txt", "--no-batch-file", "-O", "id"],
        directory.path(),
        None,
    );
    assert!(!disabled.status.success());
    assert!(!String::from_utf8_lossy(&disabled.stderr).contains("could not be read"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
