//! `-j` and `-J` lines compared exactly with saved pinned upstream output
//! (`tools/capture_info_order.py`): upstream's key order and `None` keys for
//! each represented extractor path, over the same files and headers.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    thread,
};

fn site() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/info_order/site")
}

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/info_order/cases.json")).unwrap()
}

/// Serve the fixture site with the capture's `Content-Type` and
/// `Last-Modified` headers.
fn serve(fixture: &serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let types = fixture["content_types"].as_object().unwrap().clone();
    let modified = fixture["last_modified"].as_str().unwrap().to_owned();
    let served_origin = origin.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let name = path.trim_start_matches('/').split('?').next().unwrap();
            let Ok(body) = std::fs::read(site().join(name))
                .map_err(|_| ())
                .and_then(|body| {
                    (!name.contains("..") && !name.contains('/'))
                        .then_some(body)
                        .ok_or(())
                })
            else {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            };
            let extension = name.rfind('.').map_or("", |index| &name[index..]);
            // Pages may name absolute media URLs (JSON-LD) on this server.
            let body = if matches!(extension, ".html" | ".xml") {
                String::from_utf8(body)
                    .unwrap()
                    .replace("{ORIGIN}", &served_origin)
                    .into_bytes()
            } else {
                body
            };
            let content_type = types
                .get(name)
                .or_else(|| types.get(extension))
                .and_then(serde_json::Value::as_str)
                .unwrap();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nLast-Modified: {modified}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    origin
}

/// The capture's normalization: `{ORIGIN}`/`{HOST}`, no `epoch` (this port writes no
/// `_version`).
fn normalize(line: &str, origin: &str) -> String {
    let host = origin.split_once("//").unwrap().1;
    let line = line.replace(origin, "{ORIGIN}").replace(host, "{HOST}");
    let epoch = regex::Regex::new(r#""epoch": \d+, "#).unwrap();
    epoch.replace(&line, "").into_owned()
}

#[test]
fn json_lines_match_saved_upstream_key_order_none_keys_and_downloads() {
    let fixture = fixture();
    let origin = serve(&fixture);
    let directory = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    for (case, key, flag) in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|case| [(case, "lines", "-j"), (case, "single", "-J")])
    {
        let name = case["name"].as_str().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", flag, "--user-agent", "fixture"])
            .arg(format!("{origin}/{name}"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| normalize(line, &origin))
            .collect();
        let expected: Vec<&str> = case[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect();
        if lines != expected {
            failures.push(format!(
                "{name} {flag}:\n  native   {lines:?}\n  upstream {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `--load-info-json` processes the loaded dict in place: its keys keep the
/// file's order and new keys follow, like the pinned upstream's saved
/// `-j`/`-J` lines (`tests/fixtures/info_order/loaded/`, written by upstream
/// with `--user-agent UA`; `epoch` and `_version` are not compared).
#[test]
fn loaded_info_json_keeps_upstream_key_order() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/info_order/loaded");
    let epoch = regex::Regex::new(r#""epoch": \d+, "#).unwrap();
    let version = regex::Regex::new(r#", "_version": \{[^}]*\}"#).unwrap();
    for name in ["direct", "dash"] {
        for (flag, suffix) in [("-j", "dump-json"), ("-J", "dump-single-json")] {
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args(["--ignore-config", "--load-info-json"])
                .arg(directory.join(format!("{name}.info.json")))
                .arg(flag)
                .current_dir(std::env::temp_dir())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).unwrap();
            let text = version
                .replace_all(&epoch.replace_all(&text, ""), "")
                .into_owned();
            let expected =
                std::fs::read_to_string(directory.join(format!("{name}.{suffix}.txt"))).unwrap();
            assert_eq!(text, expected, "{name} {flag}");
        }
    }
}

/// `--flat-playlist` leaves an RSS feed's `url_transparent` entries
/// unextracted: `-j` prints upstream's raw entry with its URL, playlist and
/// `_fill_common_fields(final=False)` keys, `-J` the raw entry.
#[test]
fn flat_rss_entries_are_printed_unextracted() {
    let fixture = fixture();
    let origin = serve(&fixture);
    let run = |flag: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--flat-playlist", flag])
            .arg(format!("{origin}/feed.xml"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        normalize(&String::from_utf8(output.stdout).unwrap(), &origin)
    };
    let smuggled = "{ORIGIN}/a.mp4#__youtubedl_smuggle=%7B%22force_videoid%22%3A+%22ep1%22%7D";
    let lines = run("-j");
    let first = lines.lines().next().unwrap();
    assert_eq!(
        first,
        format!(
            "{{\"_type\": \"url_transparent\", \"url\": \"{smuggled}\", \"title\": \"Ep 1\", \"description\": \"First\", \"timestamp\": 1704067200, \"duration\": 62, \"thumbnail\": null, \"episode\": null, \"episode_number\": 1, \"season_number\": 2, \"age_limit\": 0, \"__x_forwarded_for_ip\": null, \"webpage_url\": \"{smuggled}\", \"original_url\": \"{smuggled}\", \"webpage_url_basename\": \"a.mp4\", \"webpage_url_domain\": \"{{HOST}}\", \"playlist_count\": 2, \"playlist\": \"Feed\", \"playlist_id\": \"{{ORIGIN}}/feed.xml\", \"playlist_title\": \"Feed\", \"playlist_uploader\": null, \"playlist_uploader_id\": null, \"playlist_channel\": null, \"playlist_channel_id\": null, \"playlist_webpage_url\": \"{{ORIGIN}}/feed.xml\", \"n_entries\": 2, \"playlist_index\": 1, \"__last_playlist_index\": 2, \"extractor\": \"generic\", \"extractor_key\": \"Generic\", \"playlist_autonumber\": 1, \"duration_string\": \"1:02\", \"upload_date\": \"20240101\", \"release_year\": null}}"
        )
    );
    let single = run("-J");
    assert!(
        single.contains(&format!("\"entries\": [{{\"_type\": \"url_transparent\", \"url\": \"{smuggled}\", \"title\": \"Ep 1\"")),
        "{single}"
    );
}
