//! Upstream `ExternalFD` downloaders (`--downloader curl|wget|aria2c|axel|
//! httpie`) for whole-file HTTP formats, with recording fake executables.
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
            let _ = stream.read(&mut request);
            let body = b"\0\0\0\x18ftypisom native";
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

/// A fake downloader: answers the availability option, records its
/// arguments (and curl's stdin, read with shell builtins: `PATH` is empty), writes `external` to its output argument
/// and exits with `$FAKE_EXIT` (printing `boom` to stderr) if set.
fn fake(directory: &Path, name: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$1\" in -V|-v|--version) echo 'curl 8.7.1 (fake)'; exit 0;; esac\n\
         dir=\"${0%/*}\"\n\
         printf '%s\\n' \"$@\" > \"$dir/args.txt\"\n\
         if [ \"${0##*/}\" = curl ]; then while IFS= read -r line; do printf '%s\\n' \"$line\"; done > \"$dir/stdin.txt\"; fi\n\
         if [ -n \"$FAKE_EXIT\" ]; then echo boom >&2; exit \"$FAKE_EXIT\"; fi\n\
         prev=; out=; base=\n\
         for a in \"$@\"; do case \"$prev\" in -o|-O|--out|--output) out=\"$a\";; --dir) base=\"$a\";; esac; prev=\"$a\"; done\n\
         printf external > \"$base$out\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

fn run(directory: &Path, args: &[&str], url: &str, env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--user-agent", "UA", "-o", "o.%(ext)s"])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "")
        .envs(env.iter().copied())
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

const HEADERS: [&str; 8] = [
    "--header",
    "User-Agent: UA",
    "--header",
    "Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    "--header",
    "Accept-Language: en-us,en;q=0.5",
    "--header",
    "Sec-Fetch-Mode: navigate",
];

#[test]
fn curl_gets_upstream_arguments_and_cookies_on_stdin() {
    let url = format!("{}/a.mp4", serve());
    let tools = tempfile::tempdir().unwrap();
    let curl = fake(tools.path(), "curl");
    let directory = tempfile::tempdir().unwrap();
    let cookies = directory.path().join("cookies.txt");
    std::fs::write(
        &cookies,
        "# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/\tFALSE\t0\tsid\tabc\n",
    )
    .unwrap();
    let output = run(
        directory.path(),
        &[
            "-v",
            "--downloader",
            &curl,
            "--limit-rate",
            "1K",
            "--downloader-args",
            "curl:--fake",
            "--cookies",
            cookies.to_str().unwrap(),
        ],
        &url,
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let mut expected: Vec<String> = [
        "--location",
        "-o",
        "o.mp4.part",
        "--compressed",
        "--cookie",
        "-",
    ]
    .map(String::from)
    .into();
    expected.extend(HEADERS.map(String::from));
    expected.extend(
        [
            "--continue-at",
            "-",
            "--verbose",
            "--limit-rate",
            "1024",
            "--retry",
            "10",
            "--fake",
            "--",
            &url,
        ]
        .map(String::from),
    );
    assert_eq!(recorded(tools.path()), expected);
    let stdin = std::fs::read_to_string(tools.path().join("stdin.txt")).unwrap();
    assert!(
        stdin.contains("127.0.0.1\tFALSE\t/\tFALSE\t0\tsid\tabc\n"),
        "{stdin}"
    );
    assert!(!stdin.contains("# Netscape"), "{stdin}");
    assert!(
        stderr.contains(&format!("[debug] Invoking curl downloader on \"{url}\"")),
        "{stderr}"
    );
    assert!(stderr.contains("[debug] curl command line: "), "{stderr}");
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"external"
    );
}

#[test]
fn aria2c_writes_a_temporary_cookie_file_and_relative_paths() {
    let url = format!("{}/a.mp4", serve());
    let tools = tempfile::tempdir().unwrap();
    let aria2c = fake(tools.path(), "aria2c");
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &[
            "--downloader",
            &aria2c,
            "-o",
            "sub dir/o.%(ext)s",
            "--no-mtime",
        ],
        &url,
        &[],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = recorded(tools.path());
    let cookies = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--load-cookies="))
        .unwrap()
        .to_owned();
    assert!(
        stdout.contains(&format!(
            "[download] Writing temporary cookies file to \"{cookies}\"\n"
        )),
        "{stdout}"
    );
    // The temporary cookie file is removed afterwards.
    assert!(!Path::new(&cookies).exists());
    let expected = [
        "--check-certificate=true",
        "--remote-time=false",
        "--show-console-readout=true",
        "--remove-control-file=false",
        "--allow-overwrite=true",
        "--always-resume=false",
        "--auto-file-renaming=false",
        "--force-save=false",
        "--dir",
        "./sub dir/",
        "--out",
        "./o.mp4.part",
        "--",
        &url,
    ];
    assert_eq!(&args[args.len() - expected.len()..], expected);
    assert_eq!(
        std::fs::read(directory.path().join("sub dir/o.mp4")).unwrap(),
        b"external"
    );
}

#[test]
fn a_failing_downloader_reports_its_exit_code_and_stderr() {
    let url = format!("{}/a.mp4", serve());
    let tools = tempfile::tempdir().unwrap();
    let wget = fake(tools.path(), "wget");
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &["--downloader", &wget],
        &url,
        &[("FAKE_EXIT", "4")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr.contains("boom\n\n\n\nERROR: wget exited with code 4\n"),
        "{stderr:?}"
    );
    let args = recorded(tools.path());
    assert_eq!(
        &args[..5],
        [
            "-O",
            "o.mp4.part",
            "-nv",
            "--compression=auto",
            "--load-cookies"
        ]
    );
    assert!(
        args.windows(2).any(|pair| pair == ["--tries", "10"]),
        "{args:?}"
    );
}

#[test]
fn unavailable_downloaders_fall_back_to_the_native_one() {
    let url = format!("{}/a.mp4", serve());
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing/curl");
    let output = run(
        directory.path(),
        &["-v", "--downloader", missing.to_str().unwrap()],
        &url,
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("[debug] Invoking http downloader on"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"\0\0\0\x18ftypisom native"
    );
}
