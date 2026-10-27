//! Typed output templates exercised through local HTTP extraction and the CLI.
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
                let(status,mime,body)=match path.as_str(){
    "/typed.html"=>(200,"text/html","<title>Typed</title><video><source src='/media.mp4' width='1280' height='720'></video>".into()),
    "/unsafe.html"=>(200,"text/html","<title>..alpha/../../escape</title><video src='/media.mp4'></video>".into()),
    "/playlist.html"=>{let mut html="<title>Twelve entries</title>".to_owned();for i in 1..=12{html.push_str(&format!("<video src='/item/{i}.mp4'></video>"));}(200,"text/html",html)},
    "/media.mp4"=>(200,"video/mp4","media".into()),"/one.mp4"=>(200,"video/mp4","one".into()),"/12.5.mp4"=>(200,"video/mp4","numeric".into()),
    _ if path.starts_with("/item/")&&path.ends_with(".mp4")=>(200,"video/mp4",format!("entry{}",path.trim_start_matches("/item/").trim_end_matches(".mp4"))),
    _=>(404,"text/plain","missing".into())};
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
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn typed_dimensions_sizes_booleans_and_format_ids_control_downloaded_filename() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/typed.html"],
        &[
            "-o",
            "%(width)04d-%(height).1f-%(filesize)03d-%(direct)d-%(format_id)03d.%(ext)s",
        ],
        directory.path(),
    );
    success(&output);
    assert_eq!(
        // Checked with pinned upstream: HTML5 results have neither filesize
        // nor direct, so both render NA.
        std::fs::read(directory.path().join("1280-720.0-NA-NA-000.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(server.count("/media.mp4"), 2);
}

#[test]
fn numeric_string_id_converts_to_integer_and_float_while_direct_is_numeric() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/12.5.mp4"],
        &["-o", "%(id)05d-%(id).1f-%(direct)d.%(ext)s"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("00012-12.5-1.mp4")).unwrap(),
        b"numeric"
    );
}

#[test]
fn missing_and_nonnumeric_fields_use_na_defaults_and_known_alternates() {
    for (extra, placeholder) in [
        (vec![], "NA"),
        (vec!["--output-na-placeholder", "MISSING"], "MISSING"),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = vec![
            "-o",
            "%(height)06d-%(width|fallback)04d-%(height,title)s-%(ext)d-%(height|)s.%(ext)s",
        ];
        args.extend(extra);
        success(&server.cli(&["/media.mp4"], &args, directory.path()));
        let expected = format!("{placeholder}-fallback-media-{placeholder}-.mp4");
        assert_eq!(
            std::fs::read(directory.path().join(expected)).unwrap(),
            b"media"
        );
    }
}

#[test]
fn playlist_padding_uses_selected_original_max_and_selected_count_not_full_count() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "3,12",
            "-o",
            "two/%(playlist_index)s-%(playlist_autonumber)s-%(n_entries)02d.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("two/03-1-02.mp4")).unwrap(),
        b"entry3"
    );
    assert_eq!(
        std::fs::read(directory.path().join("two/12-2-02.mp4")).unwrap(),
        b"entry12"
    );
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "3",
            "-o",
            "one/%(playlist_index)s-%(playlist_autonumber)s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("one/3-1.mp4")).unwrap(),
        b"entry3"
    );
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-o",
            "all/%(playlist_index)s-%(playlist_autonumber)s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("all/01-01.mp4")).unwrap(),
        b"entry1"
    );
    assert_eq!(
        std::fs::read(directory.path().join("all/12-12.mp4")).unwrap(),
        b"entry12"
    );
    let json = server.cli(&["/playlist.html"], &["-j", "-I", "3,12"], directory.path());
    success(&json);
    for line in String::from_utf8(json.stdout).unwrap().lines() {
        let info: Value = serde_json::from_str(line).unwrap();
        assert!(info.get("last_playlist_index").is_none());
        // Upstream's dumped dict keeps the private padding key.
        assert_eq!(info["__last_playlist_index"], 12);
    }
}

#[test]
fn archive_skips_do_not_change_playlist_padding_context() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    std::fs::write(&archive, "html5mediaembed playlist-12\n").unwrap();
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "3,12",
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "%(playlist_index)s-%(playlist_autonumber)s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("03-1.mp4")).unwrap(),
        b"entry3"
    );
    assert!(!directory.path().join("3-1.mp4").exists());
    assert_eq!(server.count("/item/12.mp4"), 1);
}

#[test]
fn sanitize_before_precision_keeps_metadata_and_defaults_inside_literal_directories() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/unsafe.html"],
        &[
            "-o",
            "literal-dir/%(title).2s-%(height|../fallback).2s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("literal-dir/..-..⧸fallback.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_dir(directory.path().join("literal-dir"))
            .unwrap()
            .count(),
        1
    );
    // Metadata alone cannot form a `..` path component.
    success(&server.cli(
        &["/unsafe.html"],
        &["-o", "%(title).2s/%(id)s.%(ext)s"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read_dir(directory.path().join("_"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn unsupported_malformed_and_excessive_formats_fail_after_probe_without_download_or_archive() {
    for template in [
        "%(id)z",
        "%(id+)s",
        "%(id",
        "%(height)1000000000d",
        "%(height).9999999f",
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive.txt");
        let output = server.cli(
            &["/media.mp4"],
            &[
                "--download-archive",
                archive.to_str().unwrap(),
                "-o",
                template,
            ],
            directory.path(),
        );
        assert!(!output.status.success(), "{template}");
        assert_eq!(server.count("/media.mp4"), 1);
        assert!(!archive.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn readonly_modes_leave_invalid_template_and_placeholder_configuration_inert() {
    for mode in [vec!["-j"], vec!["-s"], vec!["-F"], vec!["-F", "-j"]] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = mode;
        args.extend([
            "-o",
            "%(unknown).9999999f",
            "--output-na-placeholder",
            "../unused",
        ]);
        let output = server.cli(&["/media.mp4"], &args, directory.path());
        success(&output);
        assert_eq!(server.count("/media.mp4"), 1);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn existing_typed_filename_is_already_downloaded_and_later_inputs_continue() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("NA.mp4"), b"keep").unwrap();
    let archive = directory.path().join("archive.txt");
    let output = server.cli(
        &["/one.mp4", "/12.5.mp4"],
        &[
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "%(title)03d.%(ext)s",
        ],
        directory.path(),
    );
    success(&output);
    assert_eq!(
        std::fs::read(directory.path().join("NA.mp4")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read(directory.path().join("012.mp4")).unwrap(),
        b"numeric"
    );
    assert_eq!(
        std::fs::read_to_string(archive).unwrap(),
        "generic one\ngeneric 12.5\n"
    );
    assert_eq!(server.count("/one.mp4"), 1);
    assert_eq!(server.count("/12.5.mp4"), 2);
}

#[test]
fn filename_option_flags_select_upstream_sanitization_and_trimming() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let filename = |args: &[&str]| {
        let mut all = vec!["-O", "filename"];
        all.extend_from_slice(args);
        let output = server.cli(&["/unsafe.html"], &all, directory.path());
        success(&output);
        String::from_utf8(output.stdout).unwrap()
    };
    let id = filename(&["-o", "%(id)s"]);
    let id = id.trim_end();
    assert_eq!(
        filename(&["-o", "%(title)s.%(ext)s"]),
        "..alpha⧸..⧸..⧸escape (1).mp4\n"
    );
    assert_eq!(
        filename(&["--restrict-filenames"]),
        format!("..alpha_.._.._escape_1-[{id}].mp4\n")
    );
    // Last flag wins, and an explicit template keeps its spaces.
    assert_eq!(
        filename(&[
            "--restrict-filenames",
            "--no-restrict-filenames",
            "-o",
            "a b.%(ext)s"
        ]),
        "a b.mp4\n"
    );
    assert_eq!(
        filename(&["--windows-filenames", "-o", "x/./y/../%(title).7s?.%(ext)s"]),
        "x/..alpha#.mp4\n"
    );
    assert_eq!(
        filename(&["--trim-file-names", "4", "-o", "%(title)s.%(ext)s"]),
        "..al.⧸escape (1).mp4\n"
    );
    if !cfg!(windows) {
        assert_eq!(
            filename(&["--no-windows-filenames", "-o", "%(title)s?.%(ext)s"]),
            "..alpha⧸..⧸..⧸escape (1)?.mp4\n"
        );
    }
    success(&server.cli(
        &["/unsafe.html"],
        &["--restrict-filenames", "--trim-filenames", "7"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(
            directory
                .path()
                .join(format!("..alpha._escape_1-[{id}].mp4"))
        )
        .unwrap(),
        b"media"
    );
}

#[test]
fn paths_and_typed_templates_place_media_and_info_json() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/typed.html"],
        &[
            "-P",
            " out ",
            "-P",
            "infojson:meta",
            "-o",
            "%(title)s.%(ext)s",
            "-o",
            "INFOJSON:j/%(title)s",
            "--write-info-json",
        ],
        directory.path(),
    ));
    let out = directory.path().join("out");
    assert_eq!(std::fs::read(out.join("Typed (1).mp4")).unwrap(), b"media");
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("meta/j/Typed (1).info.json")).unwrap())
            .unwrap();
    assert_eq!(json["title"], "Typed (1)");
    // An empty typed template skips that file, as upstream does.
    let skipped = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/typed.html"],
        &["-o", "infojson:", "--write-info-json"],
        skipped.path(),
    ));
    assert_eq!(std::fs::read_dir(skipped.path()).unwrap().count(), 1);
}

#[test]
fn templates_and_paths_expand_home_and_variables_before_rendering() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-O",
            "filename",
            "-P",
            "~/v",
            "-o",
            "$YTDLP_RS_DIR/${YTDLP_RS_DIR}/$YTDLP_RS_UNSET/%(title)s.%(ext)s",
        ])
        .arg(server.url("/typed.html"))
        .current_dir(directory.path())
        .env("HOME", "/home/fixture")
        .env("YTDLP_RS_DIR", "vars")
        .env_remove("YTDLP_RS_UNSET")
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "/home/fixture/v/vars/vars/$YTDLP_RS_UNSET/Typed (1).mp4\n"
    );
}
