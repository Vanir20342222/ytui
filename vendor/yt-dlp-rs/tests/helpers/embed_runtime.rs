//! Strict portable local subtitle-embedding process fixture.
use std::{ffi::OsString, io::Write, time::Duration};
fn main() {
    for (name, _) in std::env::vars_os() {
        assert!(
            name == "SystemRoot" || name == "WINDIR",
            "ambient environment leaked"
        );
    }
    let mode = std::fs::read_to_string("mode").unwrap();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args == ["-version"] {
        if mode == "version-hang" {
            hang();
        }
        if mode == "bad-version" {
            println!("wrong tool");
            return;
        }
        println!("ffmpeg version fixture");
        return;
    }
    let expected: Vec<OsString> = std::fs::read_to_string("expected")
        .unwrap()
        .lines()
        .map(Into::into)
        .collect();
    assert_eq!(args, expected);
    let paths: Vec<_> = args
        .windows(2)
        .filter(|pair| pair[0] == "-i")
        .map(|pair| std::path::Path::new(&pair[1]))
        .collect();
    assert_eq!(std::fs::read(paths[0]).unwrap(), b"source media");
    assert_eq!(std::fs::read(paths[1]).unwrap(), b"subtitle content");
    let output = std::path::Path::new(args.last().unwrap());
    assert_ne!(output, paths[0]);
    assert_ne!(output, paths[1]);
    match mode.as_str() {
        "hang" => hang(),
        "stderr" => {
            eprint!("fixture diagnostic");
            std::process::exit(1);
        }
        "stdout-flood" => {
            std::io::stdout().write_all(&vec![b'x'; 100000]).unwrap();
            return;
        }
        "stderr-flood" => {
            std::io::stderr().write_all(&vec![b'x'; 300000]).unwrap();
            eprint!("\nfixture tail");
            std::process::exit(1);
        }
        "fail" => std::process::exit(9),
        "missing" => return,
        "empty" => {
            std::fs::write(output, []).unwrap();
            return;
        }
        _ => {}
    }
    std::fs::write(output, b"embedded media").unwrap();
}
fn hang() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
