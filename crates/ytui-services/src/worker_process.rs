//! Blocking tools inside the supervised worker inherit its owned process group.
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
};
pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}
fn drain(mut pipe: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut overflow = false;
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let take = count.min(limit.saturating_sub(kept.len()));
        kept.extend_from_slice(&buffer[..take]);
        overflow |= take < count;
    }
    Ok((kept, overflow))
}
pub fn run(program: &str, args: &[String]) -> Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Cannot start {program}; install it or configure its path"))?;
    let stdout = child.stdout.take().context("Missing worker stdout")?;
    let stderr = child.stderr.take().context("Missing worker stderr")?;
    let out = thread::spawn(move || drain(stdout, 2 * 1024 * 1024));
    let err = thread::spawn(move || drain(stderr, 64 * 1024));
    let status = child.wait()?;
    let (stdout, overflow) = out
        .join()
        .map_err(|_| anyhow::anyhow!("Worker stdout reader failed"))??;
    let (stderr, _) = err
        .join()
        .map_err(|_| anyhow::anyhow!("Worker stderr reader failed"))??;
    ensure!(!overflow, "Worker tool output exceeded its capture limit");
    Ok(Output {
        success: status.success(),
        stdout,
        stderr: crate::process::diagnostic(&String::from_utf8_lossy(&stderr)),
    })
}
