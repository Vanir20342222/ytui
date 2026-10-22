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
async fn check(path: &std::path::Path, cancel: CancellationToken) -> Result<bool> {
    let mut spec = ProcessSpec::new(path.to_string_lossy(), vec!["--version".into()], 4);
    spec.max_output_bytes = 4096;
    let result = process::run(spec, cancel, |_| {}).await?;
    Ok(result.success && supported(&result.lines.join("\n")))
}
pub async fn select(config: &Config, cancel: CancellationToken) -> Result<Option<PathBuf>> {
    if let Some(path) = &config.native_js_runtime {
        let path = executable(&path.to_string_lossy()).ok_or_else(|| anyhow::anyhow!("Configured native JavaScript runtime was not found. Select installed Node.js 22+ or Deno 2.9.7 in Settings → Tools, or clear the field for automatic detection"))?;
        ensure!(
            check(&path, cancel).await?,
            "Native YouTube solving requires Node.js 22+ or Deno 2.9.7. The configured runtime is unsupported; update it or clear the field for automatic detection"
        );
        return Ok(Some(path));
    }
    candidates(
        ["node", "deno"]
            .into_iter()
            .filter_map(executable)
            .collect(),
        cancel,
    )
    .await
}
async fn candidates(paths: Vec<PathBuf>, cancel: CancellationToken) -> Result<Option<PathBuf>> {
    for path in paths {
        let result = check(&path, cancel.clone()).await;
        ensure!(!cancel.is_cancelled(), "Operation cancelled");
        if result.unwrap_or(false) {
            return Ok(Some(path));
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
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
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
