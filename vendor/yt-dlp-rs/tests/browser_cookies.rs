//! `--cookies-from-browser firefox` over the saved profile fixtures (see
//! tools/capture_browser_cookies.py), checked against pinned upstream.
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
            let mut request = [0; 8192];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
            );
        }
    });
    origin
}

fn profile(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/browser_cookies/firefox")
        .join(name)
}

fn run(directory: &Path, origin: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-s"])
        .args(args)
        .arg(format!("{origin}/v.mp4"))
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn firefox_cookies_load_at_the_first_request() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let spec = format!("firefox:{}::Shopping", profile("v16").display());
    let output = run(
        directory.path(),
        &origin,
        &["--cookies-from-browser", &spec, "--cookies", "jar.txt"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "[generic] v: Downloading webpage\nExtracting cookies from firefox\n\
             Extracted 1 cookies from firefox\n"
        ),
        "{stdout}"
    );
    // The jar (saved with --cookies) holds the container's cookie.
    let saved = std::fs::read_to_string(directory.path().join("jar.txt")).unwrap();
    assert!(
        saved.contains("shop.test\tFALSE\t/\tTRUE\t4102444800\tshop\ts\n"),
        "{saved}"
    );
}

#[test]
fn a_missing_database_fails_the_run_at_the_first_request() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("nope");
    let spec = format!("firefox:{}", missing.display());
    let output = run(
        directory.path(),
        &origin,
        &["--cookies-from-browser", &spec],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .ends_with("[generic] v: Downloading webpage\nExtracting cookies from firefox\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!(
            "ERROR: could not find firefox cookies database in '{}'\n",
            // Python's `repr()` of the path doubles Windows backslashes.
            missing.display().to_string().replace('\\', "\\\\")
        )
    );
    // Unsupported names are usage errors.
    let output = run(
        directory.path(),
        &origin,
        &["--cookies-from-browser", "opera2"],
    );
    assert_eq!(output.status.code(), Some(2));
}

/// Chromium on macOS: the keychain password comes from `security`.
#[cfg(target_os = "macos")]
#[test]
fn chrome_cookies_decrypt_with_the_keychain_password() {
    use std::os::unix::fs::PermissionsExt;
    let origin = serve();
    let tools = tempfile::tempdir().unwrap();
    let security = tools.path().join("security");
    std::fs::write(
        &security,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"${0%/security}/args.txt\"\nprintf 'fixture keychain password\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&security, std::fs::Permissions::from_mode(0o755)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let profile =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/browser_cookies/chrome/Default");
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-s", "--cookies", "jar.txt"])
        .arg("--cookies-from-browser")
        .arg(format!("chrome:{}", profile.display()))
        .arg(format!("{origin}/v.mp4"))
        .current_dir(directory.path())
        .env("PATH", format!("{}:/usr/bin:/bin", tools.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Extracting cookies from chrome\nExtracted 4 cookies from chrome\n")
    );
    assert_eq!(
        std::fs::read_to_string(tools.path().join("args.txt")).unwrap(),
        "find-generic-password\n-w\n-a\nChrome\n-s\nChrome Safe Storage\n"
    );
    let saved = std::fs::read_to_string(directory.path().join("jar.txt")).unwrap();
    // Raw Chromium timestamps are capped at the jar's last representable time.
    assert!(
        saved.contains(".example.test\tTRUE\t/\tTRUE\t253402300799\tsid\tsecret value\n"),
        "{saved}"
    );
    assert!(
        saved.contains("old.test\tFALSE\t/\tFALSE\t253402300799\tlegacy\tb'stored'\n"),
        "{saved}"
    );
}
