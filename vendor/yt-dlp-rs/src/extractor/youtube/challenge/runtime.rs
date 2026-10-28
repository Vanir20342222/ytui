//! Bounded process supervision. No shell, ambient Deno config, or script grants
//! (Deno without permissions, Node.js under its permission model).
use super::{Batch, MAX_STDOUT, Result, Solutions};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const ARGS: &[&str] = &[
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
    "-",
];
const PINNED_DENO: &str = "2.9.7";
const MAX_STDERR: usize = 64 * 1024;
const DEADLINE: Duration = Duration::from_secs(15);

/// Upstream `NodeJsRuntime.MIN_SUPPORTED_VERSION`.
const MIN_NODE: (u64, u64, u64) = (22, 0, 0);

pub(in crate::extractor::youtube) struct DenoRuntime {
    executable: PathBuf,
    /// The runtime's arguments before the script read from stdin.
    args: &'static [&'static str],
}

/// Upstream `NodeJCP._run_js_runtime`: the permission model (stable since
/// Node.js 23.5.0) and the script from stdin.
fn node_args(version: (u64, u64, u64)) -> &'static [&'static str] {
    if version < (23, 5, 0) {
        &[
            "--experimental-permission",
            "--no-warnings=ExperimentalWarning",
            "-",
        ]
    } else {
        &["--permission", "-"]
    }
}

/// `vMAJOR.MINOR.PATCH` (`version_tuple(..., lenient=True)`).
fn node_version(line: &str) -> Option<(u64, u64, u64)> {
    let mut parts = line.strip_prefix('v')?.split('.').map(|part| {
        part.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u64>()
    });
    let major = parts.next()?.ok()?;
    let minor = parts.next().and_then(|part| part.ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.ok()).unwrap_or(0);
    Some((major, minor, patch))
}

impl DenoRuntime {
    /// Explicit path only: discovery and installation are the caller's
    /// concern. The pinned Deno or Node.js 22+ (by its `--version` line).
    pub(in crate::extractor::youtube) fn new(executable: &Path) -> Result<Self> {
        if executable == Path::new("ytui-android-js") && crate::EMBEDDED_JS.get().is_some() {
            return Ok(Self { executable: executable.into(), args: &["embedded"] });
        }
        let executable = executable
            .canonicalize()
            .map_err(|e| format!("JavaScript runtime executable: {e}"))?;
        let output = execute(&executable, &["--version"], Vec::new(), DEADLINE, 4096)?;
        let version = std::str::from_utf8(&output).map_err(|_| "invalid runtime version output")?;
        let line = version.lines().next().unwrap_or_default();
        if line == format!("deno {PINNED_DENO} (stable, release)")
            || (line.starts_with(&format!("deno {PINNED_DENO} (stable, release, "))
                && line.ends_with(')'))
        {
            return Ok(Self {
                executable,
                args: ARGS,
            });
        }
        match node_version(line) {
            Some(version) if version >= MIN_NODE => Ok(Self {
                executable,
                args: node_args(version),
            }),
            Some(_) => Err("YouTube solving requires Node.js 22.0.0 or newer".into()),
            None => Err(format!(
                "YouTube solving requires pinned Deno {PINNED_DENO}"
            )),
        }
    }

    /// The JS challenge provider's name (`deno` or `node`).
    pub(in crate::extractor::youtube) fn name(&self) -> &'static str {
        if self.executable == Path::new("ytui-android-js") { return "android-isolate"; }
        if std::ptr::eq(self.args, ARGS) {
            "deno"
        } else {
            "node"
        }
    }

    pub(in crate::extractor::youtube) fn solve(&self, batch: &Batch) -> Result<Solutions> {
        let script = batch.script()?;
        if self.executable == Path::new("ytui-android-js") {
            let evaluator = crate::EMBEDDED_JS.get().ok_or("Embedded JS runtime is unavailable")?;
            return batch.validate(evaluator(&script)?.as_bytes());
        }

        let output = execute(
            &self.executable,
            self.args,
            script.into_bytes(),
            DEADLINE,
            MAX_STDOUT,
        )?;
        batch.validate(&output)
    }
}

enum Event {
    Input(Result<()>),
    Output(Result<Vec<u8>>),
    Error(Result<Vec<u8>>),
}

fn read_pipe(mut pipe: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = pipe
            .read(&mut buffer)
            .map_err(|e| format!("solver pipe read: {e}"))?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > cap {
            return Err("solver output budget exceeded".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn execute(
    executable: &Path,
    args: &[&str],
    input: Vec<u8>,
    deadline: Duration,
    stdout_cap: usize,
) -> Result<Vec<u8>> {
    let start = Instant::now();
    let directory = tempfile::tempdir().map_err(|e| format!("solver private directory: {e}"))?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .env_clear()
        .current_dir(directory.path())
        .env("DENO_DIR", directory.path())
        .env("DENO_NO_UPDATE_CHECK", "1")
        .env("DENO_NO_PACKAGE_JSON", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows needs these for OS/DLL lookup; none are runtime flags or caches.
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start solver runtime: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (sender, receiver) = mpsc::channel();
    // Scoped threads ensure pipes and private state cannot outlive this call.
    let result = thread::scope(|scope| {
        let tx = sender.clone();
        let spawned = thread::Builder::new().spawn_scoped(scope, move || {
            let result = stdin
                .write_all(&input)
                .map_err(|e| format!("solver stdin write: {e}"));
            drop(stdin);
            let _ = tx.send(Event::Input(result));
        });
        if let Err(error) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("solver helper thread: {error}"));
        }
        let tx = sender.clone();
        let spawned = thread::Builder::new().spawn_scoped(scope, move || {
            let _ = tx.send(Event::Output(read_pipe(stdout, stdout_cap)));
        });
        if let Err(error) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("solver helper thread: {error}"));
        }
        let spawned = thread::Builder::new().spawn_scoped(scope, move || {
            let _ = sender.send(Event::Error(read_pipe(stderr, MAX_STDERR)));
        });
        if let Err(error) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("solver helper thread: {error}"));
        }
        let mut input_done = false;
        let mut output = None;
        let mut errors = None;
        let result = loop {
            if start.elapsed() >= deadline {
                break Err("solver wall deadline exceeded".into());
            }
            match receiver.recv_timeout(
                Duration::from_millis(5).min(deadline.saturating_sub(start.elapsed())),
            ) {
                Ok(Event::Input(Ok(()))) => input_done = true,
                Ok(Event::Output(Ok(bytes))) => output = Some(bytes),
                Ok(Event::Error(Ok(bytes))) => {
                    if !bytes.is_empty() {
                        break Err(super::solver_error(&String::from_utf8_lossy(&bytes)));
                    }
                    errors = Some(bytes);
                }
                Ok(
                    Event::Input(Err(error)) | Event::Output(Err(error)) | Event::Error(Err(error)),
                ) => break Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(Duration::from_millis(5))
                }
            }
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    break Err(format!("solver exited with {status}"));
                }
                Ok(Some(_)) if input_done && output.is_some() && errors.is_some() => {
                    break Ok(output.take().unwrap());
                }
                Ok(_) => {}
                Err(error) => break Err(format!("solver process status: {error}")),
            }
        };
        if result.is_err() {
            // Killing closes child pipe ends, unblocking readers and the writer.
            // The Deno invocation grants no subprocess permission.
            let _ = child.kill();
        }
        let waited = child
            .wait()
            .map_err(|e| format!("solver process wait: {e}"));
        match (result, waited) {
            (Err(error), _) => Err(error),
            (Ok(bytes), Ok(_)) => Ok(bytes),
            (Ok(_), Err(error)) => Err(error),
        }
    });
    match (result, directory.close()) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(format!("solver private directory cleanup: {error}")),
        (Err(error), Err(cleanup)) => Err(format!(
            "{error}; solver private directory cleanup: {cleanup}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    // Compile a portable native helper, never a shell script or Python runtime.
    fn fake() -> &'static Path {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/challenge_runtime.rs");
            let binary = directory.join(format!("fake-runtime{}", std::env::consts::EXE_SUFFIX));
            let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(source)
                .arg("--edition=2024")
                .arg("-Dwarnings")
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap();
            assert!(status.success());
            binary
        })
    }

    #[test]
    fn arguments_environment_and_private_directory() {
        let output = execute(
            fake(),
            ARGS,
            b"check".to_vec(),
            Duration::from_secs(5),
            MAX_STDOUT,
        )
        .unwrap();
        let directory = PathBuf::from(String::from_utf8(output).unwrap());
        assert!(
            !directory.exists(),
            "private runtime directory must be removed"
        );
        assert!(DenoRuntime::new(fake()).is_ok());
    }

    #[test]
    fn bounded_failures_reap_child_and_join_pipes() {
        let executable = fake();
        for mode in [
            "blocked-input",
            "stdout-flood",
            "stderr-flood",
            "stderr",
            "exit",
            "drained-input-hang",
            "invalid-utf8",
        ] {
            let before = Instant::now();
            let args = [mode];
            let result = execute(
                executable,
                &args,
                vec![b'x'; 2 * 1024 * 1024],
                if mode.ends_with("hang") || mode == "blocked-input" {
                    Duration::from_millis(150)
                } else {
                    Duration::from_secs(2)
                },
                1024,
            );
            if mode == "invalid-utf8" {
                // The process boundary preserves bytes; protocol validation rejects them.
                assert_eq!(result.unwrap(), vec![0xff]);
            } else {
                let error = result.unwrap_err();
                let expected = match mode {
                    "blocked-input" | "drained-input-hang" => "deadline",
                    "stdout-flood" | "stderr-flood" => "output budget",
                    "stderr" => "diagnostic",
                    "exit" => "solver", // status or a raced broken stdin pipe
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{mode}: {error}");
            }
            assert!(before.elapsed() < Duration::from_secs(5), "{mode}");
        }
    }

    #[test]
    fn real_runtime_denies_script_permissions() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_DENO") else {
            return;
        };
        let runtime = DenoRuntime::new(Path::new(&path)).unwrap();
        let script = br#"
const actions = [
    () => Deno.env.get('DENO_DIR'),
    () => Deno.readTextFile('private-file'),
    () => Deno.writeTextFile('private-file', 'data'),
    () => fetch('http://127.0.0.1:18080/'),
    () => new Deno.Command(Deno.execPath(), {args: ['--version']}).output(),
];
for (const action of actions) {
    try { await action(); throw new Error('permission unexpectedly granted'); }
    catch (error) { if (error.name !== 'NotCapable') throw error; }
}
console.log('denied');
"#;
        let output = execute(
            &runtime.executable,
            runtime.args,
            script.to_vec(),
            DEADLINE,
            4096,
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap().trim(), "denied");
    }

    #[test]
    fn node_versions_select_upstream_permission_flags() {
        assert_eq!(node_version("v26.12.0"), Some((26, 12, 0)));
        assert_eq!(node_version("v22.1"), Some((22, 1, 0)));
        assert_eq!(node_version("deno 2.9.7"), None);
        assert_eq!(node_args((23, 4, 9))[0], "--experimental-permission");
        assert_eq!(node_args((23, 5, 0)), ["--permission", "-"]);
    }

    #[test]
    fn real_node_denies_script_permissions() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_NODE") else {
            return;
        };
        let runtime = DenoRuntime::new(Path::new(&path)).unwrap();
        let script = br#"
const actions = [
    () => require('fs').readFileSync('private-file'),
    () => require('fs').writeFileSync('private-file', 'data'),
    () => require('child_process').execFileSync(process.execPath, ['--version']),
];
for (const action of actions) {
    try { action(); throw new Error('permission unexpectedly granted'); }
    catch (error) { if (error.code !== 'ERR_ACCESS_DENIED') throw error; }
}
console.log('denied');
"#;
        let output = execute(
            &runtime.executable,
            runtime.args,
            script.to_vec(),
            DEADLINE,
            4096,
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap().trim(), "denied");
    }

    #[test]
    fn real_node_saved_player_vectors() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_NODE") else {
            return;
        };
        saved_player_vectors(&DenoRuntime::new(Path::new(&path)).unwrap());
    }

    #[test]
    fn real_saved_player_vectors() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_DENO") else {
            return;
        };
        // Dedicated CI always sets this variable; a configured runtime must pass.
        saved_player_vectors(&DenoRuntime::new(Path::new(&path)).unwrap());
    }

    fn saved_player_vectors(runtime: &DenoRuntime) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/youtube/challenges/expected.json"
        ))
        .unwrap();
        let player =
            include_str!("../../../../tests/fixtures/youtube/challenges/player-74edf1a3.js");
        let mut inputs = Vec::new();
        for request in fixture["requests"].as_array().unwrap() {
            let kind = match request["type"].as_str().unwrap() {
                "n" => super::super::Kind::N,
                "sig" => super::super::Kind::Sig,
                _ => panic!("fixture kind"),
            };
            for challenge in request["challenges"].as_object().unwrap().keys() {
                inputs.push((kind, challenge.as_str()));
            }
        }
        let batch = Batch::new(player, &inputs).unwrap();
        let solved = runtime.solve(&batch).unwrap();
        for request in fixture["requests"].as_array().unwrap() {
            let kind = if request["type"] == "n" {
                super::super::Kind::N
            } else {
                super::super::Kind::Sig
            };
            for (challenge, expected) in request["challenges"].as_object().unwrap() {
                assert_eq!(solved.get(kind, challenge), expected.as_str());
            }
        }
    }
}
