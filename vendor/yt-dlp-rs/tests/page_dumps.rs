//! `--dump-pages`/`--write-pages`: upstream `_webpage_read_content` dumps of
//! the pages and manifests extraction reads, checked against pinned upstream.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

const PAGE: &str =
    "<html><head><title>Sec</title></head><body><video src=\"v.mp4\"></video></body></html>\n";
const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,RESOLUTION=640x360\nmedia.m3u8\n";
const MEDIA: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXT-X-ENDLIST\n";

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
            let (mime, body): (&str, &[u8]) = match path.as_str() {
                "/p.html" => ("text/html", PAGE.as_bytes()),
                "/master.m3u8" => ("application/vnd.apple.mpegurl", MASTER.as_bytes()),
                "/media.m3u8" => ("application/vnd.apple.mpegurl", MEDIA.as_bytes()),
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

fn run(directory: &Path, url: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-s"])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn base64(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[test]
fn html_pages_are_dumped_and_saved_before_extracting_information() {
    let origin = serve();
    let host = origin.trim_start_matches("http://").replace(':', "_");
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &format!("{origin}/p.html"),
        &["--dump-pages", "--write-pages"],
    );
    assert!(output.status.success());
    let name = format!("p_http_-_{host}_p.html.dump");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "[generic] Extracting URL: {origin}/p.html\n[generic] p: Downloading webpage\n\
             [generic] Dumping request to {origin}/p.html\n{}\n\
             [generic] Saving request to {name}\n[generic] p: Extracting information\n\
             [info] p-1: Downloading 1 format(s): 0\n",
            base64(PAGE)
        )
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join(name)).unwrap(),
        PAGE
    );
    // Over-long names keep a prefix and an MD5 suffix (`--trim-filenames`).
    let trimmed = tempfile::tempdir().unwrap();
    let output = run(
        trimmed.path(),
        &format!("{origin}/p.html"),
        &["--write-pages", "--trim-filenames", "20"],
    );
    assert!(output.status.success());
    let names: Vec<_> = std::fs::read_dir(trimmed.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 1);
    assert!(
        names[0].starts_with("___") && names[0].ends_with(".dump"),
        "{names:?}"
    );
    assert_eq!(names[0].len(), 3 + 32 + 5);
}

#[test]
fn m3u8_manifests_dump_after_their_notes() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &format!("{origin}/master.m3u8"),
        &["--dump-pages"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "[generic] master: Downloading m3u8 information\n\
             [generic] Dumping request to {origin}/master.m3u8\n{}\n\
             [generic] master: Checking m3u8 live status\n\
             [generic] Dumping request to {origin}/media.m3u8\n{}\n",
            base64(MASTER),
            base64(MEDIA)
        )),
        "{stdout}"
    );
    // Direct media is not read as a page.
    let output = run(
        directory.path(),
        &format!("{origin}/v.mp4"),
        &["--dump-pages"],
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Dumping request"));
}

/// `--print-traffic`: the request bytes sent and the response status line and
/// headers, in `http.client`'s debug format.
#[test]
fn print_traffic_shows_requests_and_response_headers() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &format!("{origin}/v.mp4"),
        &["--print-traffic", "--user-agent", "UA/1"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let send = stdout
        .lines()
        .find(|line| line.starts_with("send: b'GET /v.mp4 HTTP/1.1\\r\\n"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(
        send.contains("user-agent: UA/1\\r\\n") && send.ends_with("\\r\\n\\r\\n'"),
        "{send}"
    );
    assert!(stdout.contains(
        "reply: 'HTTP/1.1 200 OK\\r\\n'\nheader: Content-Type: video/mp4\nheader: Content-Length: 5\nheader: Connection: close\n"
    ), "{stdout}");
}
