//! Site extractors against saved pinned-upstream output: every
//! `tests/fixtures/sites/SITE/expected.json` case (captured by
//! `tools/capture_sites.py`) runs through the fixture proxy, comparing the
//! exit code, stdout, `ERROR:`/`WARNING:` stderr lines and written files.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

fn normalize(text: &str) -> Vec<String> {
    let epoch = regex::Regex::new(r#""epoch": \d+, "#).unwrap();
    // YouTube's `available_at` is the extraction time (plus any ad wait).
    let available = regex::Regex::new(r#""available_at": \d+"#).unwrap();
    text.lines()
        .map(|line| {
            let line = epoch.replace(line, "");
            available
                .replace_all(&line, r#""available_at": 0"#)
                .into_owned()
        })
        .collect()
}

/// Whether `deno` (upstream's default JavaScript runtime) is on `PATH`.
fn deno_available() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            dir.join(format!("deno{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        })
    })
}

#[test]
fn site_extractors_match_saved_upstream_output() {
    let sites = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sites");
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&sites).unwrap() {
        let directory = entry.unwrap().path();
        let Ok(text) = std::fs::read_to_string(directory.join("expected.json")) else {
            continue;
        };
        let name = directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let proxy = site_proxy::SiteProxy::start(&name);
        let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
        for case in expected["cases"].as_array().unwrap() {
            // Cases solving YouTube challenges need upstream's default runtime.
            let requires_deno = case["requires"]
                .as_array()
                .is_some_and(|requires| requires.iter().any(|value| value == "deno"));
            if requires_deno && !deno_available() {
                continue;
            }
            let work = tempfile::tempdir().unwrap();
            let args: Vec<&str> = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap())
                .collect();
            let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    "--ignore-config",
                    "--proxy",
                    &proxy.url,
                    "--no-check-certificate",
                ])
                .args(["--user-agent", "fixture"])
                .args(&args)
                .arg(case["url"].as_str().unwrap())
                .current_dir(work.path())
                .output()
                .unwrap();
            let stderr: Vec<String> = String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter(|line| line.starts_with("ERROR:") || line.starts_with("WARNING:"))
                .map(str::to_owned)
                .collect();
            let mut files = serde_json::Map::new();
            let mut names: Vec<_> = std::fs::read_dir(work.path())
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect();
            names.sort_by_key(|entry| entry.file_name());
            for entry in names {
                files.insert(
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.metadata().unwrap().len().into(),
                );
            }
            let actual = serde_json::json!({
                "exit": output.status.code(),
                "stdout": normalize(&String::from_utf8_lossy(&output.stdout)),
                "stderr": stderr,
                "files": files,
            });
            for key in ["exit", "stdout", "stderr", "files"] {
                if actual[key] != case[key] {
                    failures.push(format!(
                        "{name} {args:?} {key}:\n  upstream {}\n  native   {}",
                        case[key], actual[key]
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn hds_downloads_assemble_flv_like_upstream_f4mfd() {
    // The FLV header, the media's metadata tag, then each fragment's
    // `mdat` payload (md5-identical to pinned upstream's file).
    let proxy = site_proxy::SiteProxy::start("f4m");
    let work = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["--no-progress", "-o", "o.%(ext)s"])
        .arg("https://media.example.test/hds/real.f4m")
        .current_dir(work.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = b"\x02\x00\x0aonMetaData\x08\x00\x00\x00\x00\x00\x00\x09";
    let mut expected = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00\x12".to_vec();
    expected.extend_from_slice(&(metadata.len() as u32).to_be_bytes()[1..]);
    expected.extend_from_slice(&[0; 7]);
    expected.extend_from_slice(metadata);
    expected.extend_from_slice(&(11 + metadata.len() as u32).to_be_bytes());
    expected.extend(b"TAG1".repeat(50));
    expected.extend(b"TAG2".repeat(50));
    assert_eq!(std::fs::read(work.path().join("o.flv")).unwrap(), expected);
}

#[test]
fn a_single_youtube_embed_is_followed_by_the_youtube_extractor() {
    // Upstream's `url_result(embed)`: the configured YouTube extractor
    // requests the watch page (which this fixture does not serve).
    let proxy = site_proxy::SiteProxy::start("youtube-embeds");
    let work = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["-s", "https://blog.example.test/one"])
        .current_dir(work.path())
        .output()
        .unwrap();
    let requests = proxy.requests.lock().unwrap();
    assert!(
        requests.iter().any(|url| url
            == "https://www.youtube.com/watch?v=BaW_jenozKc&bpctr=9999999999&has_verified=1"),
        "{requests:?}"
    );
}
