//! Upstream's derived info-dict keys (`add_default_extra_info`,
//! `_playlist_infodict`, `display_id`, HTML5 `_old_archive_ids`), checked
//! against pinned upstream.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

const PAGE: &str = "<html><head><title>Two</title></head><body>\
    <video src=\"a.mp4\"></video><video src=\"b.mp4\"></video></body></html>";

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
            let (mime, body): (&str, &[u8]) = if path == "/two.html" {
                ("text/html", PAGE.as_bytes())
            } else {
                ("video/mp4", b"media")
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

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn entries_and_results_carry_upstreams_derived_keys() {
    let origin = serve();
    let host = origin.trim_start_matches("http://");
    let directory = tempfile::tempdir().unwrap();
    let template = "%(display_id)s,%(original_url)s,%(webpage_url_basename)s,%(webpage_url_domain)s,\
        %(playlist)s,%(playlist_id)s,%(playlist_webpage_url)s,%(_old_archive_ids)s,%(__last_playlist_index)s";
    let page = format!("{origin}/two.html");
    let output = run(
        directory.path(),
        &[
            "-s",
            "--print",
            template,
            "--print",
            &format!("playlist:{template}"),
            &page,
        ],
    );
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "two-1,NA,two.html,{host},Two,two,{page},['generic two-1'],2\n\
             two-2,NA,two.html,{host},Two,two,{page},['generic two-2'],2\n\
             NA,{page},two.html,{host},NA,NA,NA,NA,NA\n"
        )
    );
    let direct = format!("{origin}/v.mp4");
    let output = run(directory.path(), &["-s", "--print", template, &direct]);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("v,{direct},v.mp4,{host},NA,NA,NA,NA,NA\n")
    );
    // `_old_archive_ids` also match download-archive records.
    std::fs::write(directory.path().join("archive.txt"), "generic two-2\n").unwrap();
    let output = run(
        directory.path(),
        &[
            "-s",
            "--download-archive",
            "archive.txt",
            "--print",
            "id",
            &page,
        ],
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "two-1\n");
}

/// Upstream `_calc_headers`: the global headers, then the format's own (an
/// HTML5 source's page Referer); merged selections have none at the top.
#[test]
fn formats_carry_their_request_headers() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let page = format!("{origin}/two.html");
    let output = run(
        directory.path(),
        &[
            "-s",
            "--user-agent",
            "UA/1",
            "--add-headers",
            "x-a:1",
            "--print",
            "%(http_headers)j",
            "-I",
            "1",
            &page,
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "{{\"User-Agent\": \"UA/1\", \"Accept\": \"text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\", \
             \"Accept-Language\": \"en-us,en;q=0.5\", \"Sec-Fetch-Mode\": \"navigate\", \"X-A\": \"1\", \"Referer\": \"{page}\"}}\n"
        )
    );
}

/// Upstream `_calc_headers` also records the format URL's cookies.
#[test]
fn formats_record_their_cookies() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("cookies.txt"),
        "# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/\tFALSE\t4102444800\tsid\tabc 123\n\
         127.0.0.1\tFALSE\t/v.mp4\tFALSE\t0\tdeep\tx\"y\n127.0.0.1\tFALSE\t/other\tFALSE\t0\tno\tpe\n",
    )
    .unwrap();
    let output = run(
        directory.path(),
        &[
            "-s",
            "--cookies",
            "cookies.txt",
            "--print",
            "%(cookies)s",
            "--print",
            "%(formats.0.cookies)s",
            &format!("{origin}/v.mp4"),
        ],
    );
    let field = "sid=\"abc 123\"; Domain=127.0.0.1; Path=/; Expires=4102444800; deep=\"x\\\"y\"; Domain=127.0.0.1; Path=/v.mp4";
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{field}\n{field}\n")
    );
}
