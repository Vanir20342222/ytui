//! Native fake solver for extraction orchestration tests; never an actual solver.
use std::{
    fs::OpenOptions,
    io::{Read, Write},
};
fn main() {
    let executable = std::env::current_exe().unwrap();
    let mode = executable.file_stem().unwrap().to_string_lossy();
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(executable.with_extension("log"))
        .unwrap();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        writeln!(log, "version").unwrap();
        println!("deno 2.9.7 (stable, release)");
        return;
    }
    writeln!(log, "solve").unwrap();
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
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let payload = input
        .rsplit_once("console.log(JSON.stringify(jsc(")
        .unwrap()
        .1;
    if mode == "malformed" {
        print!("not JSON");
        return;
    }
    if mode == "missing" {
        print!("{{\"type\":\"result\",\"responses\":[]}}");
        return;
    }
    if mode == "stderr" {
        eprintln!("solver failure");
        return;
    }
    if mode == "exit" {
        std::process::exit(7);
    }
    let n = payload.find("\"type\":\"n\"");
    let sig = payload.find("\"type\":\"sig\"");
    assert!(n.is_some() || sig.is_some());
    let mut responses = Vec::new();
    if let Some(offset) = n {
        assert!(payload[offset..].starts_with("\"type\":\"n\",\"challenges\":[\"challenge\"]"));
        responses.push((
            offset,
            "{\"type\":\"result\",\"data\":{\"challenge\":\"solved\"}}",
        ));
    }
    if let Some(offset) = sig {
        assert!(payload[offset..].starts_with("\"type\":\"sig\",\"challenges\":[\"secret\"]"));
        responses.push((
            offset,
            "{\"type\":\"result\",\"data\":{\"secret\":\"signature-value\"}}",
        ));
    }
    responses.sort_by_key(|(offset, _)| *offset);
    println!(
        "{{\"type\":\"result\",\"responses\":[{}]}}",
        responses
            .iter()
            .map(|(_, value)| *value)
            .collect::<Vec<_>>()
            .join(",")
    );
}
