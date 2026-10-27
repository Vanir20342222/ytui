//! HLS discontinuities: downloaded across like HlsFD, or split into one
//! format per section with `--hls-split-discontinuity`.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

const PLAYLIST: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\na.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:1,\nb.ts\n#EXTINF:1,\nc.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:1,\nd.ts\n#EXT-X-ENDLIST\n";

fn segment(name: &str) -> Vec<u8> {
    let mut bytes = vec![0x47];
    bytes.extend(name.bytes().cycle().take(187));
    bytes
}

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
            let (mime, body) = match path.as_str() {
                "/m.m3u8" => (
                    "application/vnd.apple.mpegurl",
                    PLAYLIST.as_bytes().to_vec(),
                ),
                _ => ("video/mp2t", segment(path.trim_start_matches('/'))),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    origin
}

fn run(directory: &Path, args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-progress",
            "--hls-use-mpegts",
            "-o",
            "o.%(ext)s",
        ])
        .args(args)
        .arg(url)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn discontinuities_download_across_or_split_into_sections() {
    let url = format!("{}/m.m3u8", serve());
    let directory = tempfile::tempdir().unwrap();
    let all: Vec<u8> = ["a.ts", "b.ts", "c.ts", "d.ts"]
        .iter()
        .flat_map(|name| segment(name))
        .collect();
    let output = run(directory.path(), &[], &url);
    assert!(stdout(&output).contains("[hlsnative] Total fragments: 4\n"));
    assert_eq!(std::fs::read(directory.path().join("o.mp4")).unwrap(), all);

    let listing = stdout(&run(
        directory.path(),
        &["-F", "--hls-split-discontinuity"],
        &url,
    ));
    let ids: Vec<&str> = listing
        .lines()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert_eq!(ids, ["0", "1", "2"]);

    // Section 1 holds b and c; HlsFD still reports every fragment.
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &[
            "--hls-split-discontinuity",
            "-f",
            "1",
            "-O",
            "after_move:%(format_index)s",
        ],
        &url,
    );
    let stdout = stdout(&output);
    assert_eq!(stdout, "1\n");
    let mut section = segment("b.ts");
    section.extend(segment("c.ts"));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        section
    );
}

#[test]
fn several_selected_formats_download_one_after_another() {
    let url = format!("{}/m.m3u8", serve());
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "--hls-use-mpegts"])
        .args([
            "--hls-split-discontinuity",
            "-f",
            "2,0",
            "-o",
            "s%(format_index)s.%(ext)s",
        ])
        .arg(&url)
        .current_dir(directory.path())
        .env("PATH", "")
        .output()
        .unwrap();
    let stdout = stdout(&output);
    assert!(
        stdout.contains("[info] m: Downloading 2 format(s): 2, 0\n"),
        "{stdout}"
    );
    assert!(
        stdout.find("Destination: s2.mp4").unwrap() < stdout.find("Destination: s0.mp4").unwrap()
    );
    assert_eq!(
        std::fs::read(directory.path().join("s2.mp4")).unwrap(),
        segment("d.ts")
    );
    assert_eq!(
        std::fs::read(directory.path().join("s0.mp4")).unwrap(),
        segment("a.ts")
    );
}
