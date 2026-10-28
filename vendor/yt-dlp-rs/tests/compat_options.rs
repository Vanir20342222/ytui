//! `--compat-options`: upstream's alias expansion and the defaults it
//! changes (`set_compat_opts`), checked against pinned upstream.
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
            let head = String::from_utf8_lossy(&request[..read]);
            let response: &[u8] = if head.starts_with("GET /three.html ") {
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html><head><title>Three</title></head><body><video src=\"a.mp4\"></video><video src=\"b.mp4\"></video><video src=\"c.mp4\"></video></body></html>"
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia"
            };
            let _ = stream.write_all(response);
        }
    });
    origin
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn compat_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find(|line| line.starts_with("[debug] Compatibility options: "))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn aliases_and_defaults_follow_set_compat_opts() {
    let origin = serve();
    let url = format!("{origin}/v.mp4");
    let directory = tempfile::tempdir().unwrap();
    // `filename` sets the default template unless -o gives one (then `*`).
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "filename",
            "-s",
            "--print",
            "filename",
            &url,
        ],
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "v-v.mp4\n");
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "filename",
            "-o",
            "x.%(ext)s",
            "-s",
            "-v",
            &url,
        ],
    );
    assert_eq!(
        compat_line(&output),
        "[debug] Compatibility options: *filename"
    );
    // Year aliases chain; `-NAME` removes.
    let output = run(
        directory.path(),
        &["--compat-options", "2024", "-s", "-v", &url],
    );
    assert_eq!(
        compat_line(&output),
        "[debug] Compatibility options: mtime-by-default"
    );
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "all,-all,abort-on-error",
            "-s",
            "-v",
            &url,
        ],
    );
    assert_eq!(
        compat_line(&output),
        "[debug] Compatibility options: abort-on-error"
    );
    let output = run(directory.path(), &["--compat-options", "bogus", "-s", &url]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .ends_with("error: wrong OPTS for --compat-options: bogus\n")
    );
    // `no-clean-infojson` keeps private keys in written info JSON.
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "no-clean-infojson",
            "--write-info-json",
            "--skip-download",
            "-o",
            "o.%(ext)s",
            &url,
        ],
    );
    assert!(output.status.success());
    let json = std::fs::read_to_string(directory.path().join("o.info.json")).unwrap();
    assert!(json.contains("\"original_url\""), "{json}");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn template_exec_extension_playlist_and_listing_options() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    // `filename-sanitization`: youtube-dl's `sanitize_filename` with `is_id`.
    let url = format!("{origin}/a:b%7Cc%20d.mp4");
    let template = "%(title)s [%(id)s].%(ext)s";
    for (compat, restrict, expected) in [
        (false, false, "a：b｜c d [a：b｜c d].mp4"),
        (true, false, "a -b_c d [a -b_c d].mp4"),
        (true, true, "a_-b_c_d [a_-b_c_d].mp4"),
    ] {
        let mut args = vec!["-s", "--print", "filename", "-o", template, &url];
        if compat {
            args.extend(["--compat-options", "filename-sanitization"]);
        }
        if restrict {
            args.push("--restrict-filenames");
        }
        assert_eq!(
            stdout(&run(directory.path(), &args)),
            format!("{expected}\n")
        );
    }
    // `allow-unsafe-exec-expansion` skips ExecPP's template validation
    // (`--exec` is unsupported on Windows).
    if !cfg!(windows) {
        let url = format!("{origin}/v.mp4");
        let command = "video:echo %(title)s >> log";
        let output = run(directory.path(), &["-s", "--exec", command, &url]);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("ERROR: Unsafe conversion(s)"));
        let output = run(
            directory.path(),
            &[
                "--compat-options",
                "allow-unsafe-exec-expansion",
                "-s",
                "--exec",
                command,
                &url,
            ],
        );
        assert!(output.status.success());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("log")).unwrap(),
            "v\n"
        );
    }
    // `allow-unsafe-ext` downloads unusual extensions, with upstream's warning.
    let url = format!("{origin}/clip.weird");
    let output = run(directory.path(), &["-o", "o.%(ext)s", &url]);
    assert!(!output.status.success());
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "allow-unsafe-ext",
            "-o",
            "o.%(ext)s",
            &url,
        ],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with(
        "WARNING: Using allow-unsafe-ext opens you up to potential attacks. Use with great care!\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("o.weird")).unwrap(),
        b"media"
    );
    // `list-formats`: the legacy table.
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "list-formats",
            "-F",
            &format!("{origin}/v.mp4"),
        ],
    );
    assert!(stdout(&output).contains("\nformat code  extension  resolution  note\n"));
    // `playlist-index`: indices follow the selection before reversing.
    let url = format!("{origin}/three.html");
    for (compat, expected) in [(false, "3 1\n1 2\n"), (true, "1 1\n3 2\n")] {
        let mut args = vec![
            "-I",
            "1,3",
            "--playlist-reverse",
            "-O",
            "%(playlist_index)s %(playlist_autonumber)s",
            &url,
        ];
        if compat {
            args.extend(["--compat-options", "playlist-index"]);
        }
        assert_eq!(stdout(&run(directory.path(), &args)), expected);
    }
}
