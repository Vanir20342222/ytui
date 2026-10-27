//! `--print`/`-O` and `--get-*` output through local HTTP extraction and the CLI.
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (done, record) = (stop.clone(), requests.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
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
                if !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    continue;
                }
                let text = String::from_utf8(request).unwrap();
                let path = text.split_whitespace().nth(1).unwrap().to_owned();
                record.lock().unwrap().push(path.clone());
                let (status, mime, body) = match path.as_str() {
                    "/page.html" => (200, "text/html", "<title>Fixture: \"A/B\" Épisode</title><video><source src='/media.mp4' width='1280' height='720'></video>".to_owned()),
                    "/playlist.html" => {
                        let mut html = "<title>Five entries</title>".to_owned();
                        for i in 1..=5 {
                            html.push_str(&format!("<video src='/item/{i}.mp4'></video>"));
                        }
                        (200, "text/html", html)
                    }
                    "/media.mp4" => (200, "video/mp4", "media".to_owned()),
                    _ if path.starts_with("/item/") && path.ends_with(".mp4") => (200, "video/mp4", "entry".to_owned()),
                    _ => (404, "text/plain", "missing".to_owned()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body.as_bytes());
            }
        });
        Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
    fn cli(&self, paths: &[&str], args: &[&str], directory: &Path) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command.args(args).current_dir(directory);
        for path in paths {
            command.arg(self.url(path));
        }
        command.output().unwrap()
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == path)
            .count()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn entries(directory: &Path) -> usize {
    std::fs::read_dir(directory).unwrap().count()
}

#[test]
fn print_templates_and_get_aliases_print_in_upstream_order_without_writing() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/page.html"],
        &[
            "--get-filename",
            "-g",
            "--get-id",
            "-e",
            "-O",
            "id,width",
            "--print",
            "VIDEO:%(title)s|%(height)05d|%(format_id|none)s|%(title)+j",
            "-O",
            "ext=",
            "--write-info-json",
            "-o",
            "out/%(title)s.%(ext)s",
        ],
        directory.path(),
    );
    let stdout = success(&output);
    let title = "Fixture: \"A/B\" Épisode (1)";
    assert_eq!(
        stdout,
        format!(
            "page-1\n1280\n{title}|00720|0|\"Fixture: \\\"A/B\\\" Épisode (1)\"\next = \"mp4\"\n{title}\npage-1\n{}\nout/Fixture： ＂A⧸B＂ Épisode (1).mp4\n",
            server.url("/media.mp4")
        )
    );
    // Quiet simulation: no diagnostics line, media download or info JSON.
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[html5]"));
    assert_eq!(entries(directory.path()), 0);
    assert_eq!(server.count("/media.mp4"), 1);
}

#[test]
fn printing_runs_per_selected_playlist_entry_before_json() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let stdout = success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "4,2",
            "-O",
            "%(playlist_index)s/%(playlist_autonumber)s/%(n_entries)s",
            "-j",
        ],
        directory.path(),
    ));
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "4/1/2");
    assert_eq!(lines[2], "2/2/2");
    for line in [lines[1], lines[3]] {
        let json: Value = serde_json::from_str(line).unwrap();
        assert_eq!(json["extractor"], "html5");
    }
    assert_eq!(entries(directory.path()), 0);
}

#[test]
fn listing_with_print_keeps_selection_and_prints_after_the_table() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let stdout = success(&server.cli(
        &["/page.html"],
        &["-F", "-O", "%(format_id)s:%(urls)s"],
        directory.path(),
    ));
    let (table, printed) = stdout.trim_end().rsplit_once('\n').unwrap();
    assert!(table.contains("ID"), "{table}");
    assert_eq!(printed, format!("0:{}", server.url("/media.mp4")));
}

#[test]
fn filename_errors_only_fail_templates_that_use_filename() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    // Checked with pinned upstream: `-o -` sends prints to stderr
    // (`logtostderr`), and the media filename is `-`.
    let output = server.cli(
        &["/page.html"],
        &["-O", "%(id)s %(filename)s", "-o", "-"],
        directory.path(),
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("\npage-1 -\n"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = server.cli(
        &["/page.html", "/media.mp4"],
        &["-e", "--get-filename", "-o", "%(width|)s"],
        directory.path(),
    );
    assert!(!output.status.success());
    // The direct file has no width, so its empty filename fails and prints nothing.
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Fixture: \"A/B\" Épisode (1)\n1280\n"
    );
    let output = server.cli(
        &["/page.html"],
        &["-e", "--get-filename", "-o", "-"],
        directory.path(),
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).ends_with("Fixture: \"A/B\" Épisode (1)\n-\n"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unsupported_print_stages_and_templates_fail_before_requests() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    for spec in ["pre_process:id", "after_filter:id", "%(id)z", "%(id+)s"] {
        let output = server.cli(&["/page.html"], &["-O", spec], directory.path());
        assert!(!output.status.success(), "{spec}");
        assert!(output.stdout.is_empty(), "{spec}");
    }
    assert_eq!(server.count("/page.html"), 0);
}

#[test]
fn print_to_file_appends_before_downloading_without_implying_simulation() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let args = [
        "--print-to-file",
        "video:id,width",
        "lists/%(extractor)s.txt",
        "--print-to-file",
        "%(filename)s",
        "names.txt",
        "-o",
        "%(id)s.%(ext)s",
    ];
    let output = server.cli(&["/page.html"], &args, directory.path());
    // Nothing is printed to stdout besides the (non-quiet) screen lines.
    let stdout = success(&output);
    assert!(
        stdout
            .lines()
            .all(|line| line.trim_start_matches('\r').starts_with('[')),
        "{stdout}"
    );
    assert!(
        stdout.contains("[info] Writing '%(id)s\\n%(width)s' to: lists/html5.txt"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("page-1.mp4")).unwrap(),
        b"media"
    );
    // A second run appends; the existing media is then already downloaded.
    let output = server.cli(&["/page.html"], &args, directory.path());
    assert!(success(&output).contains("has already been downloaded"));
    let read = |name: &str| std::fs::read_to_string(directory.path().join(name)).unwrap();
    assert_eq!(read("lists/html5.txt"), "page-1\n1280\npage-1\n1280\n");
    assert_eq!(read("names.txt"), "page-1.mp4\npage-1.mp4\n");
}

#[test]
fn print_to_file_runs_under_print_simulation_and_validates_at_startup() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "2:3",
            "-O",
            "id",
            "--print-to-file",
            "%(playlist_index)s",
            "%(extractor)s.txt",
        ],
        directory.path(),
    );
    assert_eq!(success(&output), "playlist-2\nplaylist-3\n");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[info] Writing"));
    assert_eq!(entries(directory.path()), 1);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("html5.txt")).unwrap(),
        "2\n3\n"
    );
    for args in [
        ["--print-to-file", "id", "%(id)z.txt"],
        ["--print-to-file", "%(id+)s", "x.txt"],
    ] {
        let output = server.cli(&["/page.html"], &args, directory.path());
        assert!(!output.status.success(), "{args:?}");
    }
    assert_eq!(server.count("/page.html"), 0);
}

#[test]
fn no_simulate_downloads_after_printing_json_and_listing() {
    for (flags, stdout_check) in [
        (vec!["-O", "id"], "page-1\n"),
        (vec!["-j"], "\"id\": \"page-1\""),
        (vec!["-F"], "ID"),
        (vec!["-s", "--no-simulate"], ""),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = flags.clone();
        args.extend(["--no-simulate", "-o", "%(id)s.%(ext)s"]);
        let output = server.cli(&["/page.html"], &args, directory.path());
        assert!(success(&output).contains(stdout_check), "{flags:?}");
        assert_eq!(
            std::fs::read(directory.path().join("page-1.mp4")).unwrap(),
            b"media",
            "{flags:?}"
        );
    }
    // A later -s wins over --no-simulate.
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(&["/page.html"], &["--no-simulate", "-s"], directory.path()));
    assert_eq!(entries(directory.path()), 0);
}

#[test]
fn later_print_stages_follow_upstream_order_and_disable_implied_simulation() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    // The working directory as the process sees it (macOS resolves the
    // `/var` symlink; Windows keeps the short temp path, as `abspath` does).
    let absolute = if cfg!(windows) {
        directory.path().join("page-1.mp4")
    } else {
        std::fs::canonicalize(directory.path())
            .unwrap()
            .join("page-1.mp4")
    };
    let output = server.cli(
        &["/page.html"],
        &[
            "-O",
            "after_video:%(id)s %(filepath)s",
            "-O",
            "after_move:filepath",
            "-O",
            "post_process:filepath,_filename",
            "-O",
            "Before_DL:_filename",
            "-O",
            "id",
            "--print-to-file",
            "after_move:filepath",
            "moved.txt",
            "-o",
            "%(id)s.%(ext)s",
        ],
        directory.path(),
    );
    // A later stage means the media is downloaded; printing stays quiet.
    assert_eq!(
        success(&output),
        format!(
            "page-1\npage-1.mp4\npage-1.mp4\npage-1.mp4\n{}\npage-1 NA\n",
            absolute.display()
        )
    );
    assert_eq!(
        std::fs::read(directory.path().join("page-1.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("moved.txt")).unwrap(),
        format!("{}\n", absolute.display())
    );
}

#[test]
fn get_aliases_print_upstream_forced_fields_in_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("info.json");
    let info = |extra: serde_json::Value| {
        let mut base = serde_json::json!({
            "id": "clip", "title": "Clip", "ext": "mp4", "url": "http://127.0.0.1:1/clip.mp4",
            "webpage_url": "http://127.0.0.1:1/page", "extractor": "generic", "extractor_key": "Generic",
            "formats": [{"format_id": "hd", "url": "http://127.0.0.1:1/clip.mp4", "ext": "mp4",
                         "width": 1280, "height": 720, "format_note": "main"}],
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        base
    };
    let run = |value: serde_json::Value| {
        std::fs::write(&path, value.to_string()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([
                "--get-format",
                "--get-duration",
                "--get-thumbnail",
                "--get-description",
                "--get-title",
                "--get-id",
                "--load-info-json",
            ])
            .arg(&path)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    // Upstream order: title, id, thumbnail, description, duration
    // (formatSeconds), format ('{id} - {res} (note)').
    assert_eq!(
        run(info(serde_json::json!({
            "duration": 3725, "thumbnail": "http://127.0.0.1:1/t.jpg", "description": "About"
        }))),
        "Clip\nclip\nhttp://127.0.0.1:1/t.jpg\nAbout\n1:02:05\nhd - 1280x720 (main)\n"
    );
    // Optional fields print nothing when missing.
    assert_eq!(
        run(info(serde_json::json!({}))),
        "Clip\nclip\nhd - 1280x720 (main)\n"
    );
}

#[test]
fn numeric_rate_fields_render_like_python() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("info.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "id": "clip", "title": "Clip", "ext": "mp4", "url": "http://127.0.0.1:1/v.mp4",
            "webpage_url": "http://127.0.0.1:1/page", "extractor": "generic", "extractor_key": "Generic",
            "formats": [
                {"format_id": "v", "url": "http://127.0.0.1:1/v.mp4", "ext": "mp4", "vcodec": "avc1", "acodec": "none",
                 "width": 1280, "height": 720, "tbr": 1000.0, "fps": 30},
                {"format_id": "h", "url": "http://127.0.0.1:1/h.mp4", "ext": "mp4", "vcodec": "avc1", "acodec": "none",
                 "width": 640, "height": 360, "tbr": 500.0, "fps": 29.97},
                {"format_id": "a", "url": "http://127.0.0.1:1/a.m4a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a",
                 "tbr": 96.0, "asr": 44100},
            ],
        })
        .to_string(),
    )
    .unwrap();
    let print = |format: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["-f", format, "-O"])
            .arg("%(tbr)s|%(fps)s|%(fps)j|%(aspect_ratio)05.2f|%(abr).1f|%(vbr)d|%(asr)s")
            .arg("--load-info-json")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    // Checked with pinned upstream: ints stay ints (30), floats use repr, and
    // a merge sums the rates and takes video fps/vbr and audio abr/asr.
    assert_eq!(print("v"), "1000.0|30|30|01.78|0.0|1000|NA\n");
    assert_eq!(print("h"), "500.0|29.97|29.97|01.78|0.0|500|NA\n");
    assert_eq!(print("v+a"), "1096.0|30|30|01.78|96.0|1000|44100\n");
}

#[test]
fn legacy_id_autonumber_and_deprecated_options_follow_upstream() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("info.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "id": "clip", "title": "Clip", "ext": "mp4", "url": "http://127.0.0.1:1/clip.mp4",
            "webpage_url": "http://127.0.0.1:1/page", "extractor": "generic", "extractor_key": "Generic",
        })
        .to_string(),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg("--get-filename")
            .arg("--load-info-json")
            .arg(&path)
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    // Checked with pinned upstream.
    let output = run(&["--id"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "clip.mp4\n");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("WARNING: --id is deprecated. Use -o \"%(id)s.%(ext)s\" instead")
    );
    let output = run(&["--id", "-o", "x.%(ext)s"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x.mp4\n");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("WARNING: --id is ignored since --output was given")
    );
    let output = run(&["-o", "%(autonumber)s-%(video_autonumber)s.%(ext)s"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "00001-1.mp4\n");
    let output = run(&[
        "--autonumber-start",
        "5",
        "--autonumber-size",
        "3",
        "-o",
        "%(autonumber)s %(autonumber)d.%(ext)s",
    ]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "005 5.mp4\n");
    for (args, message) in [
        (
            &["--autonumber-start", "-1"][..],
            "autonumber start \"-1\" must be positive or 0",
        ),
        (
            &["--autonumber-size", "0"][..],
            "autonumber size \"0\" must be positive",
        ),
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains(message));
    }
    let output = run(&["--call-home", "--sponskrub-args", "x", "-C"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with(
        "Deprecated Feature: The following options have been deprecated: --call-home, --sponskrub-args, -C\nPlease remove them from your command/configuration to avoid future errors.\n"
    ));
}

#[test]
fn parse_and_replace_metadata_follow_metadata_parser_pp() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("info.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "id": "clip1", "title": "Artist - Song (Live)", "ext": "mp4", "url": "http://127.0.0.1:1/clip.mp4",
            "webpage_url": "http://127.0.0.1:1/page", "extractor": "generic", "extractor_key": "Generic",
        })
        .to_string(),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg("--load-info-json")
            .arg(&path)
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    // Checked with pinned upstream.
    let output = run(&[
        "--parse-metadata",
        "title:%(artist)s - %(track)s",
        "--replace-in-metadata",
        "track,id",
        r"\s*\((\w+)\)|(\d)",
        r"[\1\2]",
        "--parse-metadata",
        "video:id:(?P<initial>.)",
        "-O",
        "%(artist)s|%(track)s|%(id)s|%(initial)s",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Artist|Song[Live]|clip[1]|c\n"
    );
    let output = run(&["--parse-metadata", "title:%(artist)s - %(track)s", "-s"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains(
        "[MetadataParser] Parsed artist from '%(title)s': 'Artist'\n[MetadataParser] Parsed track from '%(title)s': 'Song (Live)'\n"
    ));
    let output = run(&["--parse-metadata", "title", "-s"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "error: --parse-metadata title is invalid; it should be FROM:TO, not 'title'"
        )
    );
    let output = run(&["--replace-in-metadata", "title", "a", r"\q", "-s"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).ends_with("ERROR: bad escape \\q at position 0\n")
    );
}

#[test]
fn file_urls_are_refused_unless_enabled_and_then_copy_local_media() {
    let directory = tempfile::tempdir().unwrap();
    let media = directory.path().join("clip.mp4");
    std::fs::write(&media, b"\0\0\0\x18ftypisom local media").unwrap();
    let url = url::Url::from_file_path(&media).unwrap().to_string();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "-o", "out.%(ext)s"])
            .args(args)
            .arg(&url)
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    // Checked against pinned upstream.
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "ERROR: file:// URLs are disabled by default in yt-dlp for security reasons. Use --enable-file-urls to enable at your own risk.\n"
    );
    let output = run(&["--enable-file-urls"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with(
        "[info] clip: Downloading 1 format(s): mp4\n[download] Destination: out.mp4\n[download] Download completed\n"
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        std::fs::read(&media).unwrap()
    );
}
