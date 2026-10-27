//! Upstream's `playlist` stage after the entries: `--print playlist:`,
//! `--exec playlist:` and `--concat-playlist always` (FFmpegConcatPP) with
//! recording FFmpeg/ffprobe stand-ins.
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
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (mime, body): (&str, Vec<u8>) = match path.as_str() {
                "/two.html" => (
                    "text/html",
                    b"<html><head><title>Two</title></head><body><video src=\"a.mp4\"></video><video src=\"b.mp4\"></video></body></html>".to_vec(),
                ),
                _ => ("video/mp4", format!("\0\0\0\x18ftypisom {path}").into_bytes()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    origin
}

fn tools(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let ffmpeg = "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
        dir=\"${0%/ffmpeg}\"\nprev=\nfor a in \"$@\"; do\n  case \"$prev:$a\" in -i:*.concat) cp \"$a\" \"$dir/spec.txt\";; esac\n  \
        prev=\"$a\"; last=\"$a\"\ndone\nprintf 'joined' > \"$last\"\n";
    let ffprobe = "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffprobe version test'; exit 0; fi\n\
        printf '{\"streams\": [{\"codec_name\": \"h264\"}, {\"codec_name\": \"aac\"}]}'\n";
    for (name, script) in [("ffmpeg", ffmpeg), ("ffprobe", ffprobe)] {
        let path = directory.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run(directory: &Path, tools: &Path, url: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-progress",
            "-o",
            "%(title)s.%(ext)s",
        ])
        .arg("--ffmpeg-location")
        .arg(tools)
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn playlist_prints_exec_and_concatenation_follow_upstream() {
    let url = format!("{}/two.html", serve());
    let tools_dir = tempfile::tempdir().unwrap();
    tools(tools_dir.path());
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        tools_dir.path(),
        &url,
        &[
            "-s",
            "--print",
            "playlist:%(id)s %(title)s %(playlist_count)s",
            "--print",
            "%(id)s",
        ],
    );
    assert_eq!(stdout(&output), "two-1\ntwo-2\ntwo Two 2\n");

    let output = run(
        directory.path(),
        tools_dir.path(),
        &url,
        &[
            "--concat-playlist",
            "always",
            "--exec",
            "playlist:echo PL %(title)q",
        ],
    );
    let stdout = stdout(&output);
    let first = directory.path().join("Two (1).mp4");
    let second = directory.path().join("Two (2).mp4");
    let canonical = |path: &Path| {
        std::fs::canonicalize(path.parent().unwrap())
            .unwrap()
            .join(path.file_name().unwrap())
    };
    let (first_abs, second_abs) = (canonical(&first), canonical(&second));
    let expected = format!(
        "[Concat] Concatenating 2 files; Destination: Two.mp4\nDeleting original file {} (pass -k to keep)\nDeleting original file {} (pass -k to keep)\n[Exec] Executing command: echo PL Two\nPL Two\n[download] Finished downloading playlist: Two\n",
        first_abs.display(),
        second_abs.display()
    );
    // Upstream names the absolute moved paths; the temporary directory may
    // be reached through a symlink, so compare from the concat line on.
    let tail = &stdout[stdout.find("[Concat]").unwrap()..];
    let normalize = |text: &str| text.replace("/private/", "/");
    assert_eq!(normalize(tail), normalize(&expected));
    assert_eq!(
        std::fs::read(directory.path().join("Two.mp4")).unwrap(),
        b"joined"
    );
    assert!(!first.exists() && !second.exists());
    let spec = std::fs::read_to_string(tools_dir.path().join("spec.txt")).unwrap();
    let lines: Vec<&str> = spec.lines().collect();
    assert_eq!(lines[0], "ffconcat version 1.0");
    assert!(lines[1].ends_with("Two (1).mp4'") && lines[2].ends_with("Two (2).mp4'"));

    // The default policy only concatenates multi-video results.
    let directory = tempfile::tempdir().unwrap();
    let stdout = self::stdout(&run(directory.path(), tools_dir.path(), &url, &[]));
    assert!(!stdout.contains("[Concat]"));
    assert!(directory.path().join("Two (1).mp4").exists());
}
