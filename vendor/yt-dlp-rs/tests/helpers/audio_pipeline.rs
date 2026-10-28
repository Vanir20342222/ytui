//! Portable subprocess fixture for end-to-end audio publication tests.
use std::{fs, io::Write, path::PathBuf};
fn main() {
    let executable = std::env::current_exe().unwrap();
    let name = executable.file_stem().unwrap().to_str().unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(executable.with_extension("log"))
        .unwrap();
    writeln!(log, "{}", args.join("\n")).unwrap();
    let probe = name.starts_with("probe");
    if args == ["-version"] {
        println!(
            "{} version fixture",
            if probe { "ffprobe" } else { "ffmpeg" }
        );
        return;
    }
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-protocol_whitelist", "file"])
    );
    let input = PathBuf::from(&args[args.iter().position(|arg| arg == "-i").unwrap() + 1]);
    assert!(input.is_absolute());
    assert!(!fs::read(input).unwrap().is_empty());
    if probe {
        let codec = name.strip_prefix("probe-").unwrap();
        println!(
            "{{\"streams\":[{{\"codec_type\":\"video\",\"codec_name\":\"h264\"}},{{\"codec_type\":\"audio\",\"codec_name\":\"{codec}\"}}]}}"
        );
    } else {
        let output = PathBuf::from(args.last().unwrap());
        let inputs: Vec<_> = args.windows(2).filter(|pair| pair[0] == "-i").collect();
        if inputs.len() == 2 {
            for pair in inputs {
                assert_eq!(fs::read(&pair[1]).unwrap(), b"source audio");
            }
            fs::write(output, b"source audio").unwrap();
        } else {
            fs::write(output, b"converted audio").unwrap();
        }
        if name == "ffmpeg-fail" {
            std::process::exit(1);
        }
    }
}
