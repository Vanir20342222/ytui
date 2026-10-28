//! Upstream interactive format selection (`-f -`) and `--match-filters -`,
//! answered on stdin.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
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
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
            );
        }
    });
    origin
}

fn run(url: &str, input: &str) -> (Option<i32>, String, String) {
    run_with(&["-f", "-", "-s"], url, input)
}

fn run_with(args: &[&str], url: &str, input: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn selectors_are_read_until_one_selects_a_format() {
    let url = format!("{}/v.mp4", serve());
    let prompt = "\nEnter format selector (Press ENTER for default, or Ctrl+C to quit): ";
    // An unavailable and an invalid selector are reported, then asked again.
    let (code, stdout, stderr) = run(&url, "zz\nb[\nb\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("[info] Available formats for v:\n"),
        "{stdout}"
    );
    assert_eq!(stdout.matches(prompt).count(), 3, "{stdout}");
    assert!(stdout.ends_with(&format!("{prompt}[info] v: Downloading 1 format(s): mp4\n")));
    assert!(
        stderr.contains("ERROR: Requested format is not available\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("ERROR: Invalid format specification"),
        "{stderr}"
    );
    // An empty answer is the default; the end of input is an error.
    let (code, stdout, _) = run(&url, "\n");
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with("[info] v: Downloading 1 format(s): mp4\n"));
    let (code, _, stderr) = run(&url, "");
    assert_eq!(code, Some(1));
    assert!(
        stderr.ends_with("ERROR: EOF when reading a line\n"),
        "{stderr}"
    );
}

#[test]
fn interactive_match_filter_asks_before_each_download() {
    let url = format!("{}/v.mp4", serve());
    let prompt = "Download \"v [v].mp4\"? (Y/n): ";
    // Other answers ask again; `n` skips.
    let (code, stdout, _) = run_with(&["--match-filters", "-", "-s"], &url, "maybe\nn\n");
    assert_eq!(code, Some(0));
    assert!(
        stdout.ends_with(&format!(
            "[info] v: Downloading 1 format(s): mp4\n{prompt}{prompt}[download] Skipping v\n"
        )),
        "{stdout}"
    );
    // `y` (or an empty answer) downloads.
    let (code, stdout, _) = run_with(&["--match-filters", "-", "-s"], &url, "Y\n");
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with(prompt), "{stdout}");
    let (code, _, stderr) = run_with(&["--match-filters", "-", "-s"], &url, "");
    assert_eq!(code, Some(1));
    assert!(
        stderr.ends_with("ERROR: EOF when reading a line\n"),
        "{stderr}"
    );
}
