//! Portable fixture for media/subtitle embedding and preceding processors.
use std::{fs, io::Write, path::Path};
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
        .map(|pair| Path::new(&pair[1]))
        .collect();
    for input in &inputs {
        assert!(input.is_absolute());
        assert!(!fs::read(input).unwrap().is_empty());
    }
    if name.starts_with("probe") {
        println!("{{\"streams\":[{{\"codec_type\":\"audio\",\"codec_name\":\"aac\"}}]}}");
        return;
    }
    let embedding = inputs.len() > 1 && args.iter().any(|arg| arg.starts_with("-metadata:s:s:"));
    let subtitle = inputs[0]
        .extension()
        .is_some_and(|ext| ext == "sub" || ext == "srt");
    let bytes: &[u8] = if embedding {
        b"embedded media"
    } else if inputs.len() == 2 {
        b"source video"
    } else if subtitle {
        b"converted subtitle"
    } else if args.iter().any(|arg| arg == "-vn") {
        b"converted audio"
    } else {
        b"converted video"
    };
    fs::write(args.last().unwrap(), bytes).unwrap();
    if embedding && name == "ffmpeg-fail" {
        std::process::exit(7);
    }
}
