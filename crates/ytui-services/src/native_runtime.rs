//! Discover an already-installed native challenge runtime; never installs tools.
use crate::{
    paths::executable,
    process::{self, ProcessSpec},
};
use anyhow::{Result, ensure};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use ytui_core::Config;

pub fn supported(version: &str) -> bool {
    let line = version.lines().next().unwrap_or_default().trim();
    line.strip_prefix('v')
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|major| major >= 22)
        || line == "deno 2.9.7 (stable, release)"
        || (line.starts_with("deno 2.9.7 (stable, release, ") && line.ends_with(')'))
}
async fn check(path: &std::path::Path, cancel: CancellationToken) -> Result<Option<PathBuf>> {
    let mut spec = ProcessSpec::new(path.to_string_lossy(), vec!["--version".into()], 4);
    spec.max_output_bytes = 4096;
    let result = process::run(spec, cancel.clone(), |_| {}).await?;
    let version = result.lines.join("\n");
    if !result.success || !supported(&version) {
        return Ok(None);
    }
    if version.trim_start().starts_with('v') {
        // Upstream solves in a private directory. A version-manager shim may
        // work here but lose its project configuration there. Ask this exact
        // Node for its engine; do not switch the user's selected version.
        let mut spec = ProcessSpec::new(
            path.to_string_lossy(),
            vec!["-p".into(), "process.execPath".into()],
            4,
        );
        spec.max_output_bytes = 4096;
        let result = process::run(spec, cancel, |_| {}).await?;
        let engine = result.lines.first().map(PathBuf::from);
        ensure!(
            result.success
                && engine
                    .as_ref()
                    .is_some_and(|engine| engine.is_absolute() && engine.is_file()),
            "The selected Node.js runtime could not report its executable path. Select the installed Node engine in Settings → Tools"
        );
        return Ok(engine);
    }
    Ok(Some(path.to_owned()))
}
pub async fn select(config: &Config, cancel: CancellationToken) -> Result<Option<PathBuf>> {
    if let Some(path) = &config.native_js_runtime {
        let path = executable(&path.to_string_lossy()).ok_or_else(|| anyhow::anyhow!("Configured native JavaScript runtime was not found. Select installed Node.js 22+ or Deno 2.9.7 in Settings → Tools, or clear the field for automatic detection"))?;
        let engine = check(&path, cancel).await?;
        ensure!(
            engine.is_some(),
            "Native YouTube solving requires Node.js 22+ or Deno 2.9.7. The configured runtime is unsupported; update it or clear the field for automatic detection"
        );
        return Ok(engine);
    }
    candidates(
        runtime_candidates(std::env::var_os("PATH").as_deref()),
        cancel,
    )
    .await
}
fn runtime_candidates(path: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let Some(path) = path else {
        return Vec::new();
    };
    let directories = std::env::split_paths(path).take(128).collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for name in ["node", "deno"] {
        for directory in &directories {
            let candidate = directory.join(name);
            if candidate.is_file() && !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}
async fn candidates(paths: Vec<PathBuf>, cancel: CancellationToken) -> Result<Option<PathBuf>> {
    for path in paths {
        let result = check(&path, cancel.clone()).await;
        ensure!(!cancel.is_cancelled(), "Operation cancelled");
        if let Ok(Some(engine)) = result {
            return Ok(Some(engine));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn tool(root: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = -p ]; then printf '%s\n' "$0"; exit 0; fi
{body}
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[test]
    fn runtime_versions_match_the_pinned_native_solver() {
        for version in [
            "v22.0.0",
            "v24.19.0",
            "deno 2.9.7 (stable, release, x86_64-unknown-linux-gnu)",
        ] {
            assert!(supported(version));
        }
        for version in [
            "v20.19.0",
            "deno 2.9.8 (stable, release)",
            "Python 3.14",
            "",
        ] {
            assert!(!supported(version));
        }
    }
    #[tokio::test]
    async fn skips_unsupported_automatic_candidates_but_preserves_explicit_choice() {
        let temp = tempfile::tempdir().unwrap();
        let old = tool(temp.path(), "old", "echo v20.19.0");
        let node = tool(temp.path(), "node", "echo v24.19.0");
        assert_eq!(
            candidates(vec![old.clone(), node.clone()], CancellationToken::new())
                .await
                .unwrap(),
            Some(node.clone())
        );
        let mut config = Config {
            native_js_runtime: Some(old),
            ..Default::default()
        };
        assert!(
            select(&config, CancellationToken::new())
                .await
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
        config.native_js_runtime = Some(node.clone());
        assert_eq!(
            select(&config, CancellationToken::new()).await.unwrap(),
            Some(node)
        );
    }
    #[tokio::test]
    async fn an_unconfigured_shim_does_not_hide_a_supported_later_node() {
        let temp = tempfile::tempdir().unwrap();
        let shim = temp.path().join("shim");
        let system = temp.path().join("system");
        std::fs::create_dir_all(&shim).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        tool(&shim, "node", "echo 'No version configured' >&2; exit 1");
        let node = tool(&system, "node", "echo v26.8.1");
        let path = std::env::join_paths([shim.clone(), system.clone(), shim]).unwrap();
        assert_eq!(
            candidates(runtime_candidates(Some(&path)), CancellationToken::new())
                .await
                .unwrap(),
            Some(node)
        );
    }
    #[tokio::test]
    async fn a_configured_node_shim_keeps_its_engine_when_the_solver_changes_directory() {
        let temp = tempfile::tempdir().unwrap();
        let engine = tool(temp.path(), "node-engine", "echo v26.8.1");
        let shim = temp.path().join("node-shim");
        std::fs::write(&shim, format!("#!/bin/sh\nif [ \"$1\" = --version ]; then echo v26.8.1; else printf '%s\\n' '{}'; fi\n", engine.display())).unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        for explicit in [false, true] {
            let config = Config {
                native_js_runtime: Some(shim.clone()),
                ..Default::default()
            };
            let selected = if explicit {
                select(&config, CancellationToken::new()).await.unwrap()
            } else {
                candidates(vec![shim.clone()], CancellationToken::new())
                    .await
                    .unwrap()
            };
            assert_eq!(selected, Some(engine.clone()));
            assert_eq!(config.native_js_runtime.as_ref(), Some(&shim));
        }
    }
    #[tokio::test]
    async fn runtime_probe_cancellation_reaps_the_process() {
        let temp = tempfile::tempdir().unwrap();
        let slow = tool(temp.path(), "slow", "sleep 30");
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let task = tokio::spawn(async move { candidates(vec![slow], token).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
    }
}
