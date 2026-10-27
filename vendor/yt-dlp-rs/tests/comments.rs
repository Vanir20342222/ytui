//! `--write-comments`/`--get-comments`: extractors without comment support
//! only get upstream's implied `--write-info-json`.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
            );
        }
    });
    origin
}

#[test]
fn comments_imply_an_info_json_unless_json_is_printed_or_disabled() {
    let origin = serve();
    for (args, written) in [
        (&["--write-comments"][..], true),
        (&["--get-comments", "--no-write-info-json"][..], false),
        (&["--write-comments", "--no-write-comments"][..], false),
        (&["--write-comments", "-j", "--no-simulate"][..], false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
            .args(args)
            .arg(format!("{origin}/v.mp4"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
        assert!(directory.path().join("o.mp4").is_file(), "{args:?}");
        assert_eq!(
            directory.path().join("o.info.json").is_file(),
            written,
            "{args:?}"
        );
    }
}
