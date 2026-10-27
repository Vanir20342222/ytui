//! Portable process fixture for the bounded FFmpeg runner.
use std::{io::Write, time::Duration};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = std::fs::read_to_string("mode").unwrap();
    for (name, _) in std::env::vars_os() {
        assert!(
            name == "SystemRoot" || name == "WINDIR",
            "ambient environment leaked: {name:?}"
        );
    }
    if args == ["-version"] {
        if mode == "version-hang" {
            hang();
        }
        if mode == "bad-version" {
            println!("unrelated tool");
            return;
        }
        println!("ffmpeg version test-fixture");
        return;
    }
    // Match the runner's canonical paths, including Windows verbatim prefixes.
    let directory = std::env::current_dir().unwrap().canonicalize().unwrap();
    let (video_ext, video_demuxer, audio_ext, audio_demuxer, output_ext, muxer) =
        match mode.as_str() {
            "success-ts-aac" => ("ts", "mpegts", "aac", "aac", "mp4", "mp4"),
            "success-ts-m4a" => ("ts", "mpegts", "m4a", "mov", "mp4", "mp4"),
            "success-ts-ts" => ("ts", "mpegts", "ts", "mpegts", "mp4", "mp4"),
            "success-mp4-aac" => ("mp4", "mov", "aac", "aac", "mp4", "mp4"),
            "success-webm-ts" => ("webm", "matroska", "ts", "mpegts", "mkv", "matroska"),
            _ => ("mp4", "mov", "m4a", "mov", "mp4", "mp4"),
        };
    let output = directory.join(format!("output.{output_ext}"));
    let expected: Vec<std::ffi::OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-f",
        video_demuxer,
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .into_iter()
    .map(Into::into)
    .chain(std::iter::once(
        directory
            .join(format!("video.{video_ext}"))
            .into_os_string(),
    ))
    .chain(
        ["-f", audio_demuxer, "-protocol_whitelist", "file", "-i"]
            .into_iter()
            .map(Into::into),
    )
    .chain(std::iter::once(
        directory
            .join(format!("audio.{audio_ext}"))
            .into_os_string(),
    ))
    .chain(
        ["-map", "0:v:0", "-map", "1:a:0", "-c", "copy", "-f", muxer]
            .into_iter()
            .map(Into::into),
    )
    .chain(std::iter::once(output.clone().into_os_string()))
    .collect();
    assert_eq!(args, expected);
    // Stdin must be closed rather than a potentially blocking writer pipe.
    assert_eq!(
        std::io::Read::read(&mut std::io::stdin(), &mut [0]).unwrap(),
        0
    );
    match mode.as_str() {
        "stderr" => {
            eprintln!("progress noise\nfixture diagnostic\n");
            std::process::exit(1);
        }
        // Informational stderr does not fail a successful run.
        "noisy-success" => {
            eprintln!("Stream mapping: noise");
            std::fs::write(&output, b"merged").unwrap();
        }
        "exit" => std::process::exit(3),
        "empty" => {
            std::fs::write(&output, []).unwrap();
        }
        "missing" => {}
        "stdout-flood" => flood(std::io::stdout()),
        "stderr-flood" => {
            let mut stderr = std::io::stderr();
            for _ in 0..512 {
                stderr.write_all(&[b'x'; 8192]).unwrap();
            }
            eprintln!("\nflood tail");
            std::process::exit(1);
        }
        "hang" => hang(),
        "success" | "success-ts-aac" | "success-ts-m4a" | "success-ts-ts" | "success-mp4-aac"
        | "success-webm-ts" => std::fs::write(&output, b"merged").unwrap(),
        _ => panic!("unknown fixture mode"),
    }
}
fn hang() {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
fn flood(mut pipe: impl Write) {
    loop {
        if pipe.write_all(&[b'x'; 8192]).is_err() {
            return;
        }
    }
}
