//! GenericIE's `--extractor-args "generic:..."` (`fragment_query`,
//! `key_query`, `variant_query`, `hls_key`, `is_live`), as upstream
//! `_extra_manifest_info` and HlsFD apply them.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

const KEY: [u8; 16] = [0x43; 16];
const IV: [u8; 16] = [0; 16];
const PLAIN: [u8; 188] = [0x47; 188];
const PLAYLIST: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\",IV=0x00000000000000000000000000000000\n#EXTINF:1,\ns0.ts\n#EXT-X-ENDLIST\n";

fn encrypt(plain: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
    let mut buffer = vec![0; plain.len() + 16];
    buffer[..plain.len()].copy_from_slice(plain);
    cbc::Encryptor::<aes::Aes128>::new((&KEY).into(), (&IV).into())
        .encrypt_padded::<Pkcs7>(&mut buffer, plain.len())
        .unwrap()
        .to_vec()
}

/// Serves the playlist, an encrypted segment and a wrong key; records
/// request targets.
fn serve() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let record = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let target = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            record.lock().unwrap().push(target.clone());
            let path = target.split('?').next().unwrap();
            let (mime, body): (&str, Vec<u8>) = match path {
                "/v.m3u8" => ("application/vnd.apple.mpegurl", PLAYLIST.into()),
                "/s0.ts" => ("video/mp2t", encrypt(&PLAIN)),
                _ => ("application/octet-stream", vec![0; 16]),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (origin, requests)
}

fn run(directory: &Path, args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress"])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

const ARGS: &str = "generic:fragment_query=t=1;key_query=https://e.invalid/?k=2;variant_query=v=3;hls_key=0x43434343434343434343434343434343";

#[test]
fn manifest_queries_and_key_become_info_fields() {
    let (origin, _) = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &["-j", "--extractor-args", ARGS],
        &format!("{origin}/v.m3u8?m=0"),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["extra_param_to_segment_url"], "t=1");
    // `urlparse(value).query or value`.
    assert_eq!(info["extra_param_to_key_url"], "k=2");
    assert_eq!(
        info["hls_aes"],
        serde_json::json!({"key": "0x43434343434343434343434343434343"})
    );
    // `variant_query` updates every format URL's query.
    assert_eq!(info["url"], format!("{origin}/v.m3u8?m=0&v=3"));
    assert_eq!(info["live_status"], serde_json::Value::Null);
}

#[test]
fn hls_download_uses_the_fragment_query_and_external_key() {
    let (origin, requests) = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &[
            "--fixup",
            "never",
            "-o",
            "o.%(ext)s",
            "--extractor-args",
            ARGS,
        ],
        &format!("{origin}/v.m3u8"),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        PLAIN
    );
    let requests = requests.lock().unwrap();
    assert!(requests.contains(&"/s0.ts?t=1".to_owned()), "{requests:?}");
    // The `hls_aes` key replaces the playlist's (wrong) key URI.
    assert!(
        !requests.iter().any(|target| target.starts_with("/key")),
        "{requests:?}"
    );
}

#[test]
fn is_live_argument_sets_live_status_and_titles_with_the_time() {
    let (origin, _) = serve();
    let url = format!("{origin}/v.m3u8");
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &["-j", "--extractor-args", "generic:is_live=true"],
        &url,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["live_status"], "is_live");
    assert_eq!(info["is_live"], true);
    assert_eq!(info["was_live"], false);
    assert_eq!(info["fulltitle"], "v");
    let title = info["title"].as_str().unwrap();
    let suffix = title.strip_prefix("v ").unwrap();
    assert!(
        regex::Regex::new(r"^\d{4}-\d\d-\d\d \d\d:\d\d$")
            .unwrap()
            .is_match(suffix),
        "{title}"
    );
    // `--live-from-start` keeps the title.
    let output = run(
        directory.path(),
        &[
            "-j",
            "--live-from-start",
            "--extractor-args",
            "generic:is_live=true",
        ],
        &url,
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["title"], "v");
    let output = run(
        directory.path(),
        &["-j", "--extractor-args", "generic:is_live=false"],
        &url,
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["live_status"], "not_live");
    assert_eq!(info["is_live"], false);
    assert_eq!(info["title"], "v");
}

#[test]
fn malformed_extractor_args_are_usage_errors() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &["--extractor-args", "noargs"],
        "http://127.0.0.1:9/",
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "wrong --extractor-args formatting; it should be IE_KEY:ARGS, not \"noargs\""
        ),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
