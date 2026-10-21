//! Each operation owns its process group. Cancellation includes grandchildren.
use anyhow::{Context, Result, bail};
use std::{collections::VecDeque, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command, sync::mpsc};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct ProcessResult {
    pub success: bool,
    pub lines: Vec<String>,
    pub stderr: String,
}
#[derive(Clone)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub deadline: Duration,
}
impl ProcessSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>, seconds: u64) -> Self {
        Self {
            program: program.into(),
            args,
            deadline: Duration::from_secs(seconds),
        }
    }
}
enum PipeEvent {
    Line(bool, String),
    Closed,
}
async fn drain(
    mut pipe: impl tokio::io::AsyncRead + Unpin,
    stderr: bool,
    tx: mpsc::Sender<PipeEvent>,
) {
    let mut buffer = [0u8; 8192];
    let mut line = Vec::new();
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for b in &buffer[..n] {
                    if *b == b'\n' || *b == b'\r' {
                        if !line.is_empty() {
                            let s = String::from_utf8_lossy(&line).to_string();
                            line.clear();
                            if tx.send(PipeEvent::Line(stderr, s)).await.is_err() {
                                return;
                            }
                        }
                    } else if line.len() < 8 * 1024 * 1024 {
                        line.push(*b);
                    }
                }
            }
        }
    }
    if !line.is_empty() {
        let _ = tx
            .send(PipeEvent::Line(
                stderr,
                String::from_utf8_lossy(&line).to_string(),
            ))
            .await;
    }
    let _ = tx.send(PipeEvent::Closed).await;
}
#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) {
    // The group is created by us and only contains this operation's children.
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}
pub async fn run(
    spec: ProcessSpec,
    cancel: CancellationToken,
    mut on_line: impl FnMut(&str) + Send,
) -> Result<ProcessResult> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().with_context(|| {
        format!(
            "Cannot start {}; install it or set its executable path",
            spec.program
        )
    })?;
    let pid = child.id().context("Process has no PID")?;
    let (tx, mut rx) = mpsc::channel(64);
    let stdout = tokio::spawn(drain(
        child.stdout.take().context("Missing stdout")?,
        false,
        tx.clone(),
    ));
    let stderr = tokio::spawn(drain(
        child.stderr.take().context("Missing stderr")?,
        true,
        tx.clone(),
    ));
    drop(tx);
    let deadline = tokio::time::sleep(spec.deadline);
    tokio::pin!(deadline);
    let mut lines = Vec::new();
    let mut bytes = 0usize;
    let mut errors = VecDeque::new();
    let mut cancelled = false;
    let mut timed_out = false;
    let mut overflow = false;
    let mut exit_status = None;
    loop {
        tokio::select! {
            biased;
            _=cancel.cancelled()=>{cancelled=true;break;}
            _=&mut deadline=>{timed_out=true;break;}
            status=child.wait(), if exit_status.is_none()=>{
                exit_status=Some(status?);
                // Descendants must not keep inherited pipes open after their owner exits.
                #[cfg(unix)] signal_group(pid,libc::SIGKILL);
            }
            event=rx.recv()=>match event {
                Some(PipeEvent::Line(is_err,line))=> {
                    if is_err {errors.push_back(line);if errors.len()>16{errors.pop_front();}}
                    else {
                        on_line(&line);
                        if line.starts_with("YTUI_PROGRESS:") || line.starts_with("YTUI_POST:") { continue; }
                        bytes+=line.len();
                        if bytes>16*1024*1024 {overflow=true;break;}
                        lines.push(line);
                    }
                }
                Some(PipeEvent::Closed)=>{}, None=>break,
            }
        }
    }
    if cancelled || timed_out || overflow {
        #[cfg(unix)]
        signal_group(pid, libc::SIGTERM);
        #[cfg(not(unix))]
        let _ = child.start_kill();
        // Let readers drain while the group exits; escalation also kills descendants.
        let _ = tokio::time::timeout(Duration::from_millis(300), child.wait()).await;
        #[cfg(unix)]
        signal_group(pid, libc::SIGKILL);
        let _ = child.start_kill();
        let _ = child.wait().await;
        stdout.abort();
        stderr.abort();
        let _ = stdout.await;
        let _ = stderr.await;
        if cancelled {
            bail!("Operation cancelled");
        }
        if overflow {
            bail!("Tool output exceeded the 16 MiB limit");
        }
        bail!("Operation timed out");
    }
    let status = if let Some(status) = exit_status {
        status
    } else {
        tokio::select! {
            status=child.wait()=>status?,
            _=cancel.cancelled()=>{
                #[cfg(unix)] signal_group(pid,libc::SIGKILL);
                let _=child.start_kill(); let _=child.wait().await; bail!("Operation cancelled");
            }
            _=&mut deadline=>{
                #[cfg(unix)] signal_group(pid,libc::SIGKILL);
                let _=child.start_kill(); let _=child.wait().await; bail!("Operation timed out");
            }
        }
    };
    let _ = stdout.await;
    let _ = stderr.await;
    // Terminate any remaining descendants after their owning child exits.
    #[cfg(unix)]
    signal_group(pid, libc::SIGKILL);
    Ok(ProcessResult {
        success: status.success(),
        lines,
        stderr: errors.into_iter().collect::<Vec<_>>().join("\n"),
    })
}

/// Sanitize third-party errors before they reach persistent state or the terminal.
pub fn diagnostic(raw: &str) -> String {
    let raw = raw.trim().strip_prefix("ytui:").unwrap_or(raw);
    raw.split_whitespace()
        .map(|w| {
            if w.contains("http://") || w.contains("https://") {
                "[URL]".to_owned()
            } else if w.len() > 100 {
                "[redacted]".to_owned()
            } else {
                ytui_core::clean_text(w)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn drains_both_pipes_and_cancels_descendants() {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let task=tokio::spawn(run(ProcessSpec::new("sh",vec!["-c".into(),"i=0; while [ $i -lt 2000 ]; do echo out; echo err >&2; i=$((i+1)); done; sleep 30 & wait".into()],60),token,|_|{}));
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
    }
    #[tokio::test]
    async fn respects_deadline_and_collects_nonzero_exit() {
        let r = run(
            ProcessSpec::new(
                "sh",
                vec!["-c".into(), "echo test; echo failure >&2; exit 2".into()],
                2,
            ),
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert!(!r.success);
        assert_eq!(r.lines, vec!["test"]);
        assert_eq!(r.stderr, "failure");
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[tokio::test]
    async fn a_background_descendant_cannot_keep_finished_owner_pipes_open() {
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run(
                ProcessSpec::new(
                    "sh",
                    vec!["-c".into(), "sleep 30 & echo published; exit 0".into()],
                    60,
                ),
                CancellationToken::new(),
                |_| {},
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.success);
        assert_eq!(result.lines, vec!["published"]);
    }
}
