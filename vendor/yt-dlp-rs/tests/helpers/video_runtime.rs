//! Portable strict local video-tool process fixture.
use std::{ffi::OsString, io::Write, time::Duration};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = std::fs::read_to_string("mode").unwrap();
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
        if mode == "bad-version" {
            println!("unrelated executable");
            return;
        }
        println!("ffmpeg version native-test-fixture");
        return;
    }
    let directory = std::env::current_dir().unwrap().canonicalize().unwrap();
    let input = std::path::Path::new(&args[10]);
    assert_eq!(input.parent().unwrap(), directory);
    assert_eq!(std::fs::read(input).unwrap(), b"source video");
    let demuxer = match input
        .extension()
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4a" | "mov" => "mov",
        "webm" | "mkv" | "mka" => "matroska",
        "avi" => "avi",
        "flv" => "flv",
        "gif" => "gif",
        "aac" => "aac",
        "aiff" | "aif" => "aiff",
        "flac" => "flac",
        "mp3" => "mp3",
        "ogg" | "opus" | "ogv" => "ogg",
        "wav" => "wav",
        "ts" => "mpegts",
        "wma" | "wmv" => "asf",
        "mpg" | "mpeg" => "mpeg",
        _ => panic!("unknown input"),
    };
    let mut expected: Vec<OsString> = [
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
    .chain(std::iter::once(input.as_os_str().to_owned()))
    .chain(
        ["-map", "0", "-dn", "-ignore_unknown"]
            .into_iter()
            .map(Into::into),
    )
    .collect();
    let muxer = if mode == "recode" {
        "mp4"
    } else if mode == "avi" {
        expected.extend(
            ["-c:v", "libxvid", "-vtag", "XVID"]
                .into_iter()
                .map(Into::into),
        );
        "avi"
    } else {
        expected.extend(["-c", "copy"].into_iter().map(Into::into));
        "matroska"
    };
    expected.extend(["-movflags", "+faststart"].into_iter().map(Into::into));
    expected.extend(["-f", muxer].into_iter().map(Into::into));
    expected.push(args.last().unwrap().clone());
    assert_eq!(args, expected);
    let output = std::path::Path::new(args.last().unwrap());
    assert_eq!(output.parent().unwrap(), directory);
    assert_ne!(output, input);
    match mode.as_str() {
        "hang" => hang(),
        "stderr" => {
            eprint!("fixture diagnostic");
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
        "fail" => std::process::exit(7),
        "missing" => return,
        "empty" => {
            std::fs::write(output, []).unwrap();
            return;
        }
        _ => {}
    }
    std::fs::write(output, b"converted video").unwrap();
}
fn hang() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
