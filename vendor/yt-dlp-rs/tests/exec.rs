//! `--exec [WHEN:]CMD` through the CLI with a local media server; commands
//! append to a log so stage order and arguments are observable.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/clip.mp4", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
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
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
                );
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cli(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--proxy", "", "-o", "v.%(ext)s"])
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn log(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("log")).unwrap_or_default()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn stages_run_in_upstream_order_with_quoted_paths_and_fields() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let absolute = std::fs::canonicalize(directory.path())
        .unwrap()
        .join("v.mp4");
    let output = cli(
        &[
            "--exec",
            "after_video:echo after_video {} >> log",
            "--exec",
            "echo after_move {} >> log",
            "--exec",
            "POST_PROCESS:echo post_process %(filepath)q %(_filename)q %(id)q %(width)d >> log",
            "--exec",
            "before_dl:echo before_dl {} >> log",
            "--exec",
            "video:echo video {} >> log",
            &server.url,
        ],
        directory.path(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        log(directory.path()),
        format!(
            "video {{}}\nbefore_dl v.mp4\npost_process v.mp4 v.mp4 clip NA\nafter_move {}\nafter_video {{}}\n",
            absolute.display()
        )
    );
    assert!(stdout(&output).contains("[Exec] Executing command: echo before_dl v.mp4 >> log"));
}

#[test]
fn failures_follow_run_pp_and_pending_preprocessing_errors() {
    let server = Server::new();
    // A failed after_move command fails the video before archiving.
    let directory = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--exec",
            "false",
            "--download-archive",
            "archive",
            &server.url,
        ],
        directory.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("ERROR: Postprocessing: Command returned error code 1"));
    assert!(!directory.path().join("archive").exists());
    // -i reports it and continues to the next stage and the archive.
    let ignored = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "-i",
            "--exec",
            "post_process:sh -c 'exit 3'",
            "--exec",
            "echo moved >> log #",
            "--download-archive",
            "archive",
            &server.url,
        ],
        ignored.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("ERROR: Command returned error code 3\n"));
    assert_eq!(log(ignored.path()), "moved\n");
    assert!(ignored.path().join("archive").exists());
    // A before_dl failure is reported, the download continues and the error
    // is raised again afterwards.
    let pending = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--exec",
            "before_dl:false",
            "--exec",
            "echo moved >> log #",
            &server.url,
        ],
        pending.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output)
            .matches("ERROR: Preprocessing: Command returned error code 1\n")
            .count(),
        2
    );
    assert_eq!(log(pending.path()), "moved\n");
}

#[test]
fn simulation_skip_download_validation_and_resets() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "-s",
            "--exec",
            "video:echo video %(id)q >> log",
            "--exec",
            "before_dl:echo never >> log",
            "--exec",
            "echo never >> log",
            "--exec",
            "after_video:echo after_video >> log",
            &server.url,
        ],
        directory.path(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(log(directory.path()), "video clip\nafter_video\n");
    let skipped = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--skip-download",
            "--exec",
            "before_dl:echo before {} >> log",
            "--exec",
            "echo never >> log",
            "--exec",
            "one",
            "--no-exec",
            "--exec",
            "echo 100%% %(id)q >> log",
            &server.url,
        ],
        skipped.path(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    // --no-exec cleared the before_dl command too.
    assert_eq!(log(skipped.path()), "");
    assert!(!skipped.path().join("v.mp4").exists());
    for (command, message) in [
        (
            "echo %(title)s",
            "ERROR: Unsafe conversion(s) in exec command: 'echo %(title)s'\nConversions such as %()s are too dangerous",
        ),
        (
            "echo %(title|a b)q",
            "ERROR: Unsafe default(s) in exec command: 'echo %(title|a b)q'",
        ),
        (
            "pre_process:echo",
            "ERROR: --exec stage \"pre_process\" is unsupported",
        ),
    ] {
        let output = cli(&["--exec", command, &server.url], directory.path());
        assert_eq!(output.status.code(), Some(1), "{command}");
        assert!(
            stderr(&output).starts_with(message),
            "{command}: {}",
            stderr(&output)
        );
    }
    let output = cli(
        &[
            "--exec-before-download",
            "echo old >> log #",
            "--exec",
            "before_dl:echo new >> log #",
            &server.url,
        ],
        skipped.path(),
    );
    assert!(output.status.success());
    assert!(stderr(&output).contains(
        "WARNING: --exec-before-download is ignored since \"--exec before_dl:\" was given"
    ));
    assert_eq!(log(skipped.path()), "new\n");
}

#[test]
fn use_postprocessor_exec_runs_before_exec_commands_of_its_stage() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--exec",
            "after_move:echo exec >> log #",
            "--use-postprocessor",
            "ExecPP:exec_cmd=echo added %(id)q >> log;when=after_move",
            "--use-postprocessor",
            "Exec:exec_cmd=echo default >> log #",
            &server.url,
        ],
        directory.path(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(log(directory.path()), "default\nadded clip\nexec\n");
    for (value, message) in [
        (
            "FFmpegMetadata",
            "postprocessor \"FFmpegMetadata\" is not available natively",
        ),
        (
            "Exec:when=video",
            "the Exec postprocessor needs exec_cmd=COMMAND",
        ),
        ("Exec:oops", "invalid --use-postprocessor argument \"oops\""),
    ] {
        let output = cli(
            &["--use-postprocessor", value, &server.url],
            directory.path(),
        );
        assert!(!output.status.success(), "{value}");
        assert!(
            stderr(&output).contains(message),
            "{value}: {}",
            stderr(&output)
        );
    }
}
