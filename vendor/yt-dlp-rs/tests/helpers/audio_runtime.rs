//! Portable process fixture for explicit local-only audio tools.
use std::{
    ffi::OsString,
    io::{Read, Write},
    time::Duration,
};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = std::fs::read_to_string("mode").unwrap();
    for (name, _) in std::env::vars_os() {
        assert!(
            name == "SystemRoot" || name == "WINDIR",
            "ambient environment leaked: {name:?}"
        );
    }
    let probe = std::env::current_exe()
        .unwrap()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .contains("probe");
    if args == ["-version"] {
        if mode == "version-hang" {
            hang();
        }
        if mode == "bad-version" {
            println!("unrelated executable");
            return;
        }
        println!(
            "{} version test-fixture",
            if probe { "ffprobe" } else { "ffmpeg" }
        );
        return;
    }
    let directory = std::env::current_dir().unwrap().canonicalize().unwrap();
    let input_index = args.iter().position(|arg| arg == "-i").unwrap() + 1;
    let input = std::path::Path::new(&args[input_index]);
    assert_eq!(input.parent().unwrap(), directory);
    assert_eq!(std::fs::read(input).unwrap(), b"source audio");
    let demuxer = match input.extension().unwrap().to_str().unwrap() {
        "mp4" | "m4a" | "mov" => "mov",
        "webm" | "mkv" | "mka" => "matroska",
        "mp3" => "mp3",
        "aac" => "aac",
        "ogg" | "opus" => "ogg",
        "wav" => "wav",
        "flac" => "flac",
        "ts" => "mpegts",
        "avi" => "avi",
        "flv" => "flv",
        "aiff" | "aif" => "aiff",
        _ => panic!("unknown fixture input"),
    };
    let mut expected: Vec<OsString> = if probe {
        [
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,codec_type",
            "-of",
            "json",
            "-protocol_whitelist",
            "file",
            "-f",
            demuxer,
            "-i",
        ]
        .into_iter()
        .map(Into::into)
        .collect()
    } else {
        [
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "repeat+info",
            "-y",
            "-f",
            demuxer,
            "-protocol_whitelist",
            "file",
            "-i",
        ]
        .into_iter()
        .map(Into::into)
        .collect()
    };
    expected.push(input.as_os_str().into());
    let (output, muxer) = match mode.as_str() {
        "encode" => ("output.mp3", "mp3"),
        "default-codec" => ("output.wav", "wav"),
        _ => ("output.m4a", "ipod"),
    };
    if !probe {
        expected.extend(["-map", "0:a:0", "-vn"].into_iter().map(OsString::from));
        if mode != "default-codec" {
            expected.extend(
                [
                    "-acodec",
                    if mode == "encode" {
                        "libmp3lame"
                    } else {
                        "copy"
                    },
                ]
                .into_iter()
                .map(OsString::from),
            );
        }
        if mode == "encode" {
            expected.extend(["-q:a", "5"].into_iter().map(OsString::from));
        }
        expected.extend([
            OsString::from("-f"),
            muxer.into(),
            directory.join(output).into_os_string(),
        ]);
    }
    assert_eq!(args, expected);
    assert_eq!(std::io::stdin().read(&mut [0]).unwrap(), 0);
    match mode.as_str() {
        "stderr" => {
            eprintln!("fixture diagnostic");
            std::process::exit(1);
        }
        "exit" => std::process::exit(3),
        "stdout-flood" => flood(std::io::stdout()),
        "stderr-flood" => {
            std::io::stderr().write_all(&vec![b'x'; 300_000]).unwrap();
            eprint!("\nfixture tail");
            std::process::exit(1);
        }
        "hang" => hang(),
        "no-audio" => println!(r#"{{"streams":[{{"codec_type":"video","codec_name":"h264"}}]}}"#),
        "invalid-json" => println!("not JSON"),
        "missing-codec" => println!(r#"{{"streams":[{{"codec_type":"audio"}}]}}"#),
        "invalid-codec" => {
            println!(r#"{{"streams":[{{"codec_type":"audio","codec_name":"--options"}}]}}"#)
        }
        "empty" => std::fs::write(output, []).unwrap(),
        "missing" => {}
        _ if probe => println!(
            r#"{{"programs":[],"stream_groups":[],"streams":[{{"codec_type":"video","codec_name":"h264"}},{{"codec_type":"audio","codec_name":"aac"}},{{"codec_type":"audio","codec_name":"mp3"}}]}}"#
        ),
        "copy" | "encode" | "default-codec" => std::fs::write(output, b"processed audio").unwrap(),
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
