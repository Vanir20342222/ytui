//! `-v`/`--verbose`: upstream `write_debug` lines for configuration,
//! extraction, format selection, downloads and metadata parsing.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
    time::Duration,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
            }
            let text = String::from_utf8_lossy(&request);
            let path = text.split_whitespace().nth(1).unwrap_or("/");
            let (mime, body): (&str, &[u8]) = match path {
                "/page.html" => (
                    "text/html",
                    b"<html><head><title>Page</title></head><body><video src=\"v.mp4\"></video></body></html>",
                ),
                _ => ("video/mp4", b"media"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

fn run(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .current_dir(directory)
        .env_remove("http_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("https_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("all_proxy")
        .env_remove("ALL_PROXY")
        .output()
        .unwrap()
}

fn debug_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.starts_with("[debug] "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn verbose_reports_configuration_selection_and_download() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let url = format!("{origin}/v.mp4");
    let output = run(
        &[
            "-v",
            "--no-progress",
            "-o",
            "o.%(ext)s",
            "--download-archive",
            "archive.txt",
            "-u",
            "name",
            "--password=secret",
            &url,
        ],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines = debug_lines(&output);
    assert_eq!(
        lines[0],
        format!(
            "[debug] Command-line config: ['--ignore-config', '-v', '--no-progress', '-o', 'o.%(ext)s', '--download-archive', 'archive.txt', '-u', 'PRIVATE', '--password=PRIVATE', '{url}']"
        )
    );
    assert!(
        lines[1].starts_with(
            "[debug] Encodings: locale utf-8, fs utf-8, pref utf-8, out utf-8 (No ANSI)"
        )
    );
    for expected in [
        "[debug] Proxy map: {}".to_owned(),
        "[debug] Loaded 62 extractors".to_owned(),
        "[debug] Loading archive file 'archive.txt'".to_owned(),
        "[debug] Identified a direct video link".to_owned(),
        "[debug] Formats sorted by: hasvid, ie_pref, lang, quality, res, fps, hdr:12(7), vcodec, channels, acodec, size, br, asr, proto, vext, aext, hasaud, source, id".to_owned(),
        format!("[debug] Invoking http downloader on \"{url}\""),
        "[debug] Adding to archive: generic v".to_owned(),
    ] {
        assert!(lines.contains(&expected), "{expected} not in {lines:#?}");
    }
    let spec = lines
        .iter()
        .position(|line| line.starts_with("[debug] Default format spec: "))
        .unwrap();
    let invoking = lines
        .iter()
        .position(|line| line.starts_with("[debug] Invoking"))
        .unwrap();
    assert!(spec < invoking);
    // Untruncated URLs and the normal screen output on stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("[generic] Extracting URL: {url}")));
    assert!(stdout.contains("[download] Destination: o.mp4"));
}

#[test]
fn verbose_reports_sort_orders_embeds_and_metadata_parsing() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        &[
            "-v",
            "-s",
            "-S",
            "res:720,+size",
            "--parse-metadata",
            "title:%(artist)s - %(track)s",
            "--replace-in-metadata",
            "title",
            "(P)",
            "x\\1",
            &format!("{origin}/page.html"),
        ],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines = debug_lines(&output);
    for expected in [
        "[debug] Looking for embeds",
        "[debug] Identified a html5 embed",
        "[debug] Sort order given by user: res:720, +size",
        "[debug] Formats sorted by: hasvid, ie_pref, res:720(720.0), +size, lang, quality, fps, hdr:12(7), vcodec, channels, acodec, br, asr, proto, vext, aext, hasaud, source, id",
        r"[debug] Searching for '(?P<artist>.+)\\ \\-\\ (?P<track>.+)' in '%(title)s'",
        "[debug] Replacing all '(P)' in title with 'x\\\\1'",
        "[debug] Default format spec: bestvideo*+bestaudio/best",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected} not in {lines:#?}"
        );
    }
}

#[test]
fn verbose_overrides_quiet_and_no_warnings_on_stderr() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let url = format!("{origin}/v.mp4");
    let output = run(
        &["-v", "-q", "--no-warnings", "-s", "-f", "best", &url],
        directory.path(),
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Screen messages move to stderr; warnings become debug lines.
    assert!(stderr.contains(&format!("\n[generic] Extracting URL: {url}\n")));
    assert!(stderr.contains("\n[debug] WARNING: \"-f best\" selects the best pre-merged format which is often not the best option.\n"));
    assert!(!stderr.contains("\nWARNING:"));
    // Without --verbose, quiet mode and --no-warnings stay silent.
    let output = run(
        &["-q", "--no-warnings", "-s", "-f", "best", &url],
        directory.path(),
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[test]
fn common_format_mistakes_warn_like_upstream() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let url = format!("{origin}/v.mp4");
    let output = run(&["-s", "-f", "mp4", &url], directory.path());
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "WARNING: \"-f mp4\" selects the best pre-merged mp4 format which is often not what's intended.\n         Pre-merged mp4 formats are not available from all sites, or may only be available in lower quality.\n         To prioritize the best h264 video and aac audio in an mp4 container, use \"-t mp4\" instead.\n         If you know what you are doing and want a pre-merged mp4 format, use \"-f b[ext=mp4]\" instead to suppress this warning\n"
    );
    // Validation warnings keep upstream's order.
    let output = run(
        &[
            "-s",
            "-f",
            "best",
            "--date",
            "20200101",
            "--dateafter",
            "20190101",
            "--playlist-random",
            "--playlist-reverse",
            &url,
        ],
        directory.path(),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("WARNING:"))
        .collect();
    assert_eq!(
        warnings,
        [
            "WARNING: \"-f best\" selects the best pre-merged format which is often not the best option.",
            "WARNING: --playlist-reverse is ignored since --playlist-random was given",
            "WARNING: --dateafter is ignored since --date was given",
        ]
    );
}
