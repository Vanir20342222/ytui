//! Upstream postprocessor progress hooks (`--progress-template postprocess:`)
//! and `--console-title` escape codes.
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
    format!("{origin}/a.mp4")
}

fn command(directory: &Path, args: &[&str], url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command
        .args(["--ignore-config", "-o", "o.%(ext)s"])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "");
    command
}

fn run(directory: &Path, args: &[&str], url: &str) -> Output {
    command(directory, args, url).output().unwrap()
}

const TEMPLATE: &str = "postprocess:PP %(info.id)s %(progress.postprocessor)s %(progress.status)s";

#[test]
#[cfg_attr(windows, ignore = "--exec is unsupported on Windows")]
fn postprocessor_hooks_surround_each_processor_run() {
    let url = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &[
            "--no-progress",
            "--progress-template",
            TEMPLATE,
            "--parse-metadata",
            "title:%(artist)s",
            "--exec",
            "before_dl:echo before",
            "--exec",
            "echo after",
        ],
        &url,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.starts_with("[generic]") && !line.starts_with("[info]"))
        .collect();
    assert_eq!(
        lines,
        [
            "PP a MetadataParser started",
            "[MetadataParser] Parsed artist from '%(title)s': 'a'",
            "PP a MetadataParser finished",
            "PP a Exec started",
            "[Exec] Executing command: echo before o.mp4",
            "before o.mp4",
            "PP a Exec finished",
            "[download] Destination: o.mp4",
            "[download] Download completed",
            "PP a MoveFiles started",
            "PP a MoveFiles finished",
            "PP a Exec started",
            &format!(
                "[Exec] Executing command: echo after {}",
                directory
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("o.mp4")
                    .display()
            ),
            &format!(
                "after {}",
                directory
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("o.mp4")
                    .display()
            ),
            "PP a Exec finished",
        ][..]
            .iter()
            .map(|line| line.as_ref())
            .collect::<Vec<&str>>(),
        "{stdout}"
    );
}

#[test]
fn postprocessor_templates_print_when_quiet_and_without_downloads() {
    let url = serve();
    let directory = tempfile::tempdir().unwrap();
    // `to_screen(..., quiet=False)` writes to the quiet screen (stderr).
    let output = run(
        directory.path(),
        &["-q", "--no-download", "--progress-template", TEMPLATE],
        &url,
    );
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "PP a MoveFiles started\nPP a MoveFiles finished\n"
    );
    // Simulation runs no processors.
    let output = run(
        directory.path(),
        &["-s", "--progress-template", TEMPLATE],
        &url,
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PP "));
}

#[test]
fn console_title_needs_a_terminal() {
    let url = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path(), &["--console-title"], &url)
        .env("TERM", "xterm")
        .output()
        .unwrap();
    assert!(output.status.success());
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !stream.contains(&0x1b),
            "{}",
            String::from_utf8_lossy(stream)
        );
    }
}

#[cfg(unix)]
mod pty {
    use super::*;
    use std::{
        ffi::{CStr, c_char, c_int},
        fs::File,
        os::fd::FromRawFd,
    };

    unsafe extern "C" {
        fn posix_openpt(flags: c_int) -> c_int;
        fn grantpt(fd: c_int) -> c_int;
        fn unlockpt(fd: c_int) -> c_int;
        fn ptsname(fd: c_int) -> *const c_char;
    }

    /// Run with stderr on a pseudo terminal and return what it received.
    fn on_terminal(mut command: Command) -> (std::process::ExitStatus, String) {
        // SAFETY: plain libc calls on a descriptor this function owns.
        let (master, name) = unsafe {
            let fd = posix_openpt(2); // O_RDWR
            assert!(fd >= 0);
            assert_eq!(grantpt(fd), 0);
            assert_eq!(unlockpt(fd), 0);
            let name = CStr::from_ptr(ptsname(fd)).to_string_lossy().into_owned();
            (File::from_raw_fd(fd), name)
        };
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        let mut child = command
            .env("TERM", "xterm")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::from(slave))
            .spawn()
            .unwrap();
        let reader = thread::spawn(move || {
            let mut master = master;
            let mut received = Vec::new();
            let mut buffer = [0; 4096];
            // The master reads EOF/EIO once the child closes the terminal.
            while let Ok(read) = master.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..read]);
            }
            received
        });
        let mut stdout = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut stdout)
            .unwrap();
        let status = child.wait().unwrap();
        // Closing our copy of the slave lets the reader finish.
        drop(command);
        let received = reader.join().unwrap();
        (status, String::from_utf8_lossy(&received).into_owned())
    }

    /// The OSC/CSI title and progress codes in `text`.
    fn codes(text: &str) -> Vec<String> {
        regex::Regex::new("\x1b\\][^\x07]*\x07|\x1b\\[[0-9;]*t")
            .unwrap()
            .find_iter(text)
            .map(|found| found.as_str().replace('\x1b', "ESC").replace('\x07', "BEL"))
            .collect()
    }

    #[test]
    fn console_title_follows_download_and_postprocessor_progress() {
        let url = serve();
        let directory = tempfile::tempdir().unwrap();
        let (status, received) = on_terminal(command(
            directory.path(),
            &[
                "--console-title",
                "--progress-template",
                "download-title:D %(progress._percent_str)s",
            ],
            &url,
        ));
        assert!(status.success(), "{received}");
        let codes = codes(&received);
        assert_eq!(
            codes,
            [
                "ESC[22;0t",
                "ESC]9;4;3;0BEL",
                "ESC]0;D 100.0%BEL",
                "ESC]9;4;1;100BEL",
                // The finished status (`format_percent(100)`).
                "ESC]0;D 100.0%BEL",
                "ESC]9;4;3;100BEL",
                "ESC]0;yt-dlp MoveFiles startedBEL",
                "ESC]9;4;3;0BEL",
                "ESC]0;yt-dlp MoveFiles finishedBEL",
                "ESC]9;4;3;0BEL",
                "ESC[23;0t",
                "ESC]9;4;0;0BEL",
            ],
            "{received:?}"
        );
        // Simulation neither saves nor restores the title.
        let (_, received) =
            on_terminal(command(directory.path(), &["--console-title", "-s"], &url));
        assert_eq!(
            super::pty::codes(&received),
            ["ESC]9;4;3;0BEL", "ESC]9;4;0;0BEL"]
        );
    }
}

#[test]
#[cfg_attr(windows, ignore = "--exec is unsupported on Windows")]
fn metadata_parsing_runs_at_later_stages_after_their_prints() {
    let url = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &[
            "--no-progress",
            "--parse-metadata",
            "before_dl:%(title)s-b:%(artist)s",
            "--parse-metadata",
            "post_process:%(artist)s-p:%(artist)s",
            "--parse-metadata",
            "after_move:%(artist)s-m:%(artist)s",
            "--parse-metadata",
            "after_video:%(title)s-v:%(album)s",
            "--print",
            "post_process:P %(artist)s",
            "--print",
            "after_move:M %(artist)s",
            "--exec",
            "after_move:echo EM %(artist)q",
            "--print",
            "after_video:V %(artist)s %(album)s",
            "--exec",
            "after_video:echo EV %(artist)q %(album)q",
        ],
        &url,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // `--print` implies `--quiet`, which hides the processors' screen lines.
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [
            "P a-b",
            "M a-b-p",
            "EM a-b-p-m",
            // `after_video` has the info dict from before the download.
            "V NA NA",
            "EV NA a-v",
        ],
        "{stdout}"
    );
}

/// A page with two videos (an HTML5 playlist) titled `Two`.
fn serve_playlist() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (mime, body): (&str, &[u8]) = if path == "/two.html" {
                (
                    "text/html",
                    b"<html><head><title>Two</title></head><body><video src=\"a.mp4\"></video><video src=\"b.mp4\"></video></body></html>",
                )
            } else {
                ("video/mp4", b"\0\0\0\x18ftypisom media")
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    format!("{origin}/two.html")
}

#[test]
#[cfg_attr(windows, ignore = "--exec is unsupported on Windows")]
fn playlist_metadata_parsing_follows_playlist_prints() {
    let url = serve_playlist();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &[
            "-s",
            "--replace-in-metadata",
            "playlist:title",
            "T",
            "Z",
            "--print",
            "playlist:P %(title)s",
            "--exec",
            "playlist:echo E %(title)q",
        ],
        &url,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.ends_with("P Two\nE Zwo\n"), "{stdout}");
}
