//! Compiled by the challenge supervisor tests as a native fake runtime.
use std::{
    io::{Read, Write},
    time::Duration,
};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("deno 2.9.7 (stable, release)\nv8 fake\ntypescript fake");
        return;
    }
    let mode = args.first().unwrap().as_str();
    // Isolate output-budget tests from stdin failure races: a reader hitting
    // its cap can close the pipe and make the helper exit before stdin drains.
    if matches!(mode, "stdout-flood" | "stderr-flood") {
        std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
    }
    match mode {
        "blocked-input" => std::thread::sleep(Duration::from_secs(30)),
        "drained-input-hang" => {
            std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "stdout-flood" => loop {
            if std::io::stdout().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        "stderr-flood" => loop {
            if std::io::stderr().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        "stderr" => {
            std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
            eprintln!("diagnostic");
        }
        "exit" => std::process::exit(7),
        "invalid-utf8" => {
            std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
            std::io::stdout().write_all(&[0xff]).unwrap();
        }
        "run" => {
            assert_eq!(
                args,
                [
                    "run",
                    "--ext=js",
                    "--no-code-cache",
                    "--no-prompt",
                    "--no-remote",
                    "--no-lock",
                    "--node-modules-dir=none",
                    "--no-config",
                    "--no-npm",
                    "--cached-only",
                    "-"
                ]
            );
            let cwd = std::env::current_dir().unwrap();
            assert_eq!(
                std::path::PathBuf::from(std::env::var_os("DENO_DIR").unwrap())
                    .canonicalize()
                    .unwrap(),
                cwd.canonicalize().unwrap()
            );
            for (key, _) in std::env::vars_os() {
                assert!(
                    [
                        "DENO_DIR",
                        "DENO_NO_UPDATE_CHECK",
                        "DENO_NO_PACKAGE_JSON",
                        "NO_COLOR",
                        "SystemRoot",
                        "WINDIR"
                    ]
                    .iter()
                    .any(|name| key == *name),
                    "unexpected environment {:?}",
                    key
                );
            }
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            assert_eq!(input, "check");
            print!("{}", cwd.display());
        }
        _ => panic!("unknown mode"),
    }
}
