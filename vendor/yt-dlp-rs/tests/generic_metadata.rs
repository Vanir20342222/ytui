//! Generic page metadata through the CLI: `Last-Modified` timestamps with the
//! derived `upload_date`, OpenGraph description/image and RTA age limits.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
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
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (mime, modified, body): (&str, &str, &[u8]) = match path.as_str() {
                    "/meta.html" => (
                        "text/html",
                        "Wed, 21 Oct 2015 07:28:00 GMT",
                        include_bytes!("fixtures/html5/meta.html"),
                    ),
                    "/plain.html" => (
                        "text/html",
                        "not a date",
                        b"<title>Plain</title><video poster='/p.jpg' src='/media/clip.mp4'></video>",
                    ),
                    "/media/clip.mp4" => ("video/mp4", "Sun, 06 Nov 1994 08:49:37 GMT", b"media"),
                    _ => ("text/plain", "", b"missing"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nLast-Modified: {modified}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        Self {
            origin,
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

fn json(url: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--proxy", "", "-j", url])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn page_and_direct_metadata_follow_upstream_generic_fields() {
    let server = Server::new();
    let page = json(&format!("{}/meta.html", server.origin));
    assert_eq!(page["timestamp"], 1445412480);
    assert_eq!(page["upload_date"], "20151021");
    assert_eq!(page["description"], "Cats & dogs");
    assert_eq!(page["age_limit"], 18);
    // og:image fills a missing poster verbatim (not resolved against the page).
    assert_eq!(page["thumbnail"], "images/og.jpg");
    let direct = json(&format!("{}/media/clip.mp4", server.origin));
    assert_eq!(direct["timestamp"], 784111777);
    assert_eq!(direct["upload_date"], "19941106");
    assert!(direct.get("description").is_none());
    // Unparsable dates are absent; posters win over og:image.
    let plain = json(&format!("{}/plain.html", server.origin));
    assert!(plain.get("timestamp").is_none() && plain.get("upload_date").is_none());
    assert_eq!(plain["thumbnail"], format!("{}/p.jpg", server.origin));
}

/// Exit code, stdout (screen lines) and stderr (warnings and errors).
fn simulate(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--proxy", "", "-s"])
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn date_age_view_and_title_selection_follow_upstream_match_entry() {
    let server = Server::new();
    let page = format!("{}/meta.html", server.origin);
    let direct = format!("{}/media/clip.mp4", server.origin);
    for (args, url, reason) in [
        (
            vec!["--date", "20151022"],
            &page,
            Some("[download] 2015-10-21 upload date is not in range 2015-10-22 to 2015-10-22"),
        ),
        (
            vec!["--datebefore", "19941105"],
            &direct,
            Some("[download] 1994-11-06 upload date is not in range 0001-01-01 to 1994-11-05"),
        ),
        (
            vec!["--dateafter", "19941106", "--datebefore", "19941106"],
            &direct,
            None,
        ),
        (
            vec!["--age-limit", "17"],
            &page,
            Some("[download] Skipping \"Meta page (1)\" because it is age restricted"),
        ),
        (vec!["--age-limit", "18"], &page, None),
        // Unknown content ratings and view counts never reject.
        (vec!["--age-limit", "0", "--min-views", "5"], &direct, None),
        (vec!["--match-title", "META"], &page, None),
        (
            vec!["--match-title", "zzz"],
            &page,
            Some("[download] \"Meta page (1)\" title did not match pattern \"zzz\""),
        ),
        (
            vec!["--reject-title", "p.ge"],
            &page,
            Some("[download] \"Meta page (1)\" title matched reject pattern \"p.ge\""),
        ),
    ] {
        let (code, stdout, stderr) = simulate(&[args.as_slice(), &[url.as_str()]].concat());
        assert_eq!(code, Some(0), "{args:?}: {stderr}");
        match reason {
            Some(reason) => assert!(stdout.contains(reason), "{args:?}: {stdout}"),
            None => assert!(!stdout.contains("[download] "), "{args:?}: {stdout}"),
        }
    }
    // --break-on-reject turns any rejection into upstream's cancellation.
    let (code, stdout, _) = simulate(&["--break-on-reject", "--age-limit", "1", &page]);
    assert_eq!(code, Some(101));
    assert!(stdout.contains(
        "[download] Skipping \"Meta page (1)\" because it is age restricted\n[info] Encountered a video that did not match filter, stopping due to --break-match-filter\nAborting remaining downloads"
    ), "{stdout}");
    // --date wins over --dateafter/--datebefore with upstream warnings.
    let (code, _, stderr) = simulate(&[
        "--dateafter",
        "20990101",
        "--datebefore",
        "20000101",
        "--date",
        "19941106",
        &direct,
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stderr.contains("WARNING: --dateafter is ignored since --date was given\nWARNING: --datebefore is ignored since --date was given"), "{stderr}");
    // Invalid dates fail before any request with upstream messages.
    for (value, message) in [
        ("bogus", "Invalid date format \"bogus\""),
        ("20201301", "unconverted data remains: 1"),
        (
            "20200001",
            "time data '20200001' does not match format '%Y%m%d'",
        ),
    ] {
        let (code, _, stderr) = simulate(&["--date", value, "http://127.0.0.1:9/never"]);
        assert_eq!(code, Some(2));
        assert!(
            stderr.ends_with(&format!("yt-dlp-rs: error: {message}\n")),
            "{value}: {stderr}"
        );
    }
    let (_, _, stderr) = simulate(&[
        "--dateafter",
        "20200102",
        "--datebefore",
        "20200101",
        &direct,
    ]);
    assert!(stderr.contains(
        "Date range: \"2020-01-02 to 2020-01-01\" , the start date must be before the end date"
    ));
}

fn modified(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn mtime_sets_media_modification_times_from_last_modified() {
    let server = Server::new();
    let url = format!("{}/media/clip.mp4", server.origin);
    for (args, expected) in [
        (vec!["--mtime"], true),
        (vec!["--mtime", "--no-mtime"], false),
        (vec![], false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(directory.path())
            .args(["--proxy", "", "-o", "clip.%(ext)s"])
            .args(&args)
            .arg(&url)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
        assert_eq!(
            modified(&directory.path().join("clip.mp4")) == 784_111_777,
            expected,
            "{args:?}"
        );
    }
}

#[test]
fn generic_extraction_reports_upstream_screen_lines() {
    let server = Server::new();
    // Screen lines go to stdout and warnings to stderr, like upstream.
    let lines = |args: &[&str], path: &str| -> (Vec<String>, Vec<String>) {
        let (code, stdout, stderr) =
            simulate(&[args, &[format!("{}{path}", server.origin).as_str()]].concat());
        assert_eq!(code, Some(0), "{stderr}");
        let normalize = |text: &str| -> Vec<String> {
            text.lines()
                .filter(|line| line.starts_with("[generic]") || line.starts_with("WARNING"))
                .map(|line| line.replace(&server.origin, "<O>"))
                .collect()
        };
        (normalize(&stdout), normalize(&stderr))
    };
    let warning = "WARNING: [generic] Falling back on generic information extractor";
    let (screen, warnings) = lines(&[], "/media/clip.mp4");
    assert_eq!(
        screen,
        [
            "[generic] Extracting URL: <O>/media/clip.mp4",
            "[generic] clip: Downloading webpage",
        ]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let page_screen = [
        "[generic] Extracting URL: <O>/meta.html",
        "[generic] meta: Downloading webpage",
        "[generic] meta: Extracting information",
    ];
    assert_eq!(
        lines(&[], "/meta.html"),
        (
            page_screen.map(String::from).to_vec(),
            vec![warning.to_owned()]
        )
    );
    assert_eq!(
        lines(&["--no-warnings"], "/meta.html"),
        (page_screen.map(String::from).to_vec(), Vec::new())
    );
    assert_eq!(
        lines(&["-q"], "/meta.html"),
        (Vec::new(), vec![warning.to_owned()])
    );
    // `--force-generic-extractor` words it as forcing.
    assert_eq!(
        lines(&["--force-generic-extractor"], "/meta.html").1,
        ["WARNING: [generic] Forcing generic information extractor"]
    );
}

#[test]
fn sleep_requests_waits_before_later_extraction_requests() {
    let server = Server::new();
    let page = format!("{}/meta.html", server.origin);
    let direct = format!("{}/media/clip.mp4", server.origin);
    let (code, stdout, stderr) = simulate(&["--sleep-requests", "0.2", &direct, &page]);
    assert_eq!(code, Some(0), "{stderr}");
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("[generic]"))
        .map(|line| line.replace(&server.origin, "<O>"))
        .collect();
    // The first request of the run is not delayed; HTML5 media sources are
    // probed without a delay, as upstream does not request them.
    assert_eq!(
        lines,
        [
            "[generic] Extracting URL: <O>/media/clip.mp4",
            "[generic] clip: Downloading webpage",
            "[generic] Extracting URL: <O>/meta.html",
            "[generic] Sleeping 0.2 seconds ...",
            "[generic] meta: Downloading webpage",
            "[generic] meta: Extracting information",
        ]
    );
    let (code, _, stderr) = simulate(&["--sleep-requests", "-1", &direct]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("requests sleep interval \"-1.0\" must be positive or 0"));
}
