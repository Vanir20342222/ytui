//! Strict native process fixture for subtitle FFmpeg transport.
use std::{ffi::OsString, io::Write, path::Path, time::Duration};
fn hang() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = if std::env::current_exe().unwrap().file_stem().unwrap() == "ffmpeg-fail" {
        "fail"
    } else {
        "success"
    };
    for (name, _) in std::env::vars_os() {
        assert!(
            name == "SystemRoot" || name == "WINDIR",
            "ambient environment leaked"
        );
    }
    if args == ["-version"] {
        if mode == "version-hang" {
            hang();
        }
        println!(
            "{}",
            if mode == "bad-version" {
                "unrelated executable"
            } else {
                "ffmpeg version subtitle-fixture"
            }
        );
        return;
    }
    let input = Path::new(&args[10]);
    let output = Path::new(args.last().unwrap());
    let directory = std::env::current_dir().unwrap().canonicalize().unwrap();
    assert_eq!(input.parent().unwrap(), directory);
    assert_eq!(output.parent().unwrap(), directory);
    assert_ne!(input, output);
    assert!(!std::fs::read(input).unwrap().is_empty());
    let demux = args[6].to_str().unwrap();
    assert!(["webvtt", "ass", "srt", "lrc"].contains(&demux));
    let mux = match output.extension().unwrap().to_str().unwrap() {
        "vtt" => "webvtt",
        "srt" => "srt",
        "ass" => "ass",
        "lrc" => "lrc",
        _ => panic!("output format"),
    };
    let expected: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-f",
        demux,
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .into_iter()
    .map(Into::into)
    .chain(std::iter::once(input.as_os_str().to_owned()))
    .chain(
        ["-map", "0:s:0", "-movflags", "+faststart", "-f", mux]
            .into_iter()
            .map(Into::into),
    )
    .chain(std::iter::once(output.as_os_str().to_owned()))
    .collect();
    assert_eq!(args, expected);
    match mode {
        "hang" => hang(),
        "fail" => std::process::exit(7),
        "missing" => return,
        "stderr" => {
            eprint!("subtitle diagnostic");
            std::process::exit(1);
        }
        "large-stderr" => {
            std::io::stderr().write_all(&vec![b'x'; 300_000]).unwrap();
            eprint!("\nfixture tail");
            std::process::exit(1);
        }
        "large-stdout" => {
            std::io::stdout().write_all(&vec![b'x'; 100_000]).unwrap();
            return;
        }
        "empty" => std::fs::write(output, []).unwrap(),
        "invalid-utf8" => std::fs::write(output, [0xff]).unwrap(),
        "large-output" => std::fs::File::create(output)
            .unwrap()
            .set_len(16 * 1024 * 1024 + 1)
            .unwrap(),
        _ => std::fs::write(output, b"converted subtitle").unwrap(),
    }
}
