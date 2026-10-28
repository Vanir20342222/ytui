//! Portable ffmpeg/ffprobe fixture for composed video processing.
use std::{fs, io::Write};
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
    if args == ["-version"] {
        println!(
            "{} version fixture",
            if name.starts_with("probe") {
                "ffprobe"
            } else {
                "ffmpeg"
            }
        );
        return;
    }
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-protocol_whitelist", "file"])
    );
    let inputs: Vec<_> = args
        .windows(2)
        .filter(|pair| pair[0] == "-i")
        .map(|pair| std::path::Path::new(&pair[1]))
        .collect();
    for input in &inputs {
        assert!(input.is_absolute());
        assert!(!fs::read(input).unwrap().is_empty());
    }
    if name.starts_with("probe") {
        println!("{{\"streams\":[{{\"codec_type\":\"audio\",\"codec_name\":\"aac\"}}]}}");
        return;
    }
    let output = args.last().unwrap();
    let audio = args.iter().any(|arg| arg == "-vn");
    let bytes: &[u8] = if inputs.len() == 2 {
        b"source video"
    } else if audio {
        b"converted audio"
    } else {
        b"converted video"
    };
    fs::write(output, bytes).unwrap();
    if name == "ffmpeg-fail" && !audio {
        std::process::exit(1);
    }
}
