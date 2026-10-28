//! `--postprocessor-args`/`--ppa`: arguments by postprocessor key, before
//! each input and the output file, with a recording FFmpeg stand-in.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
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

/// Records its arguments (one per line) in `args.txt` and writes the last
/// argument as the output file.
fn fake_ffmpeg(directory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"${0%/ffmpeg}/args.txt\"\n\
         for a in \"$@\"; do last=\"$a\"; done\nprintf 'remuxed' > \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn run(directory: &Path, ffmpeg: &Path, args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .arg("--ffmpeg-location")
        .arg(ffmpeg)
        .args(["--remux-video", "mkv"])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn recorded(tools: &Path) -> Vec<String> {
    std::fs::read_to_string(tools.join("args.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn postprocessor_args_follow_configuration_args_keys() {
    let origin = serve();
    let url = format!("{origin}/v.mp4");
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &ffmpeg,
        &[
            "--ppa",
            "VideoRemuxer+ffmpeg_i1:-in 'one value'",
            "--postprocessor-args",
            "videoremuxer+FFMPEG_o:-out",
            "--ppa",
            "ffmpeg:-unused",
            "--ppa",
            "Merger:-other",
        ],
        &url,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let args = recorded(tools.path());
    let input = args.iter().position(|arg| arg == "-i").unwrap();
    assert_eq!(args[input - 2..input], ["-in", "one value"]);
    assert_eq!(args[args.len() - 2], "-out");
    assert!(!args.iter().any(|arg| arg == "-unused" || arg == "-other"));

    // Unnamed arguments warn and become the `default` output arguments;
    // the postprocessor and `ffmpeg` keys take precedence and concatenate.
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), &ffmpeg, &["--ppa", "-dflt"], &url);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "WARNING: Post-Processor arguments given without specifying name. The arguments will be given to all post-processors\n"
    );
    let args = recorded(tools.path());
    assert_eq!(args[args.len() - 2], "-dflt");
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &ffmpeg,
        &[
            "--ppa",
            "-dflt",
            "--ppa",
            "VideoRemuxer:-pp",
            "--ppa",
            "ffmpeg:-exe",
        ],
        &url,
    );
    assert!(output.status.success());
    let args = recorded(tools.path());
    assert_eq!(args[args.len() - 3..args.len() - 1], ["-pp", "-exe"]);
    assert!(!args.iter().any(|arg| arg == "-dflt"));
}

#[test]
fn malformed_postprocessor_args_are_usage_errors() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--ppa",
            "Merger:\"open",
            "-s",
            "http://127.0.0.1:9/v.mp4",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .ends_with("error: wrong --ppa formatting; No closing quotation\n")
    );
}

#[test]
fn unnamed_downloader_args_warn_like_report_args_compat() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let warning = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "-s"])
            .args(args)
            .arg(format!("{origin}/v.mp4"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    assert_eq!(
        warning(&["--downloader-args", "-v"]),
        "WARNING: External Downloader arguments given without specifying name. The arguments will be given to all external downloaders\n"
    );
    assert_eq!(
        warning(&["--downloader", "ffmpeg", "--downloader-args", "-v"]),
        "WARNING: External Downloader arguments given without specifying name. The arguments will be given to ffmpeg\n"
    );
    assert_eq!(warning(&["--downloader-args", "ffmpeg:-v"]), "");
}
