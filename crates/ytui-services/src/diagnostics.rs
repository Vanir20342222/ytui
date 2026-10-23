//! Explicit allowlist: support exports never serialize a config, session,
//! environment, media item, path, stream URL, title or third-party error text.
use crate::{
    native_runtime, paths,
    process::{self, ProcessSpec},
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use ytui_core::Config;
#[derive(Clone, Debug, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub state: &'static str,
    pub version: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Support {
    pub schema: u32,
    pub version: String,
    pub revision: String,
    pub os: &'static str,
    pub architecture: &'static str,
    pub playback_backend: String,
    pub active_backend: Option<&'static str>,
    pub download_backend: String,
    pub proxy_scheme: String,
    pub failure_phase: &'static str,
    pub failure_phase_source: &'static str,
    pub tools: Vec<Tool>,
}
pub fn phase(message: &str) -> &'static str {
    let text = message.to_ascii_lowercase();
    let has = |parts: &[&str]| parts.iter().any(|part| text.contains(part));
    if message.is_empty() {
        "none"
    } else if has(&["cannot start", "runtime", "executable"]) {
        "tool"
    } else if has(&["signature", "challenge", "cipher", "javascript"]) {
        "challenge"
    } else if has(&[
        "bootstrap",
        "consent",
        "player responses",
        "sign in",
        "bot",
        "session",
    ]) {
        "bootstrap/account"
    } else if has(&["format", "eligible media"]) {
        "format"
    } else if has(&[
        "pipewire",
        "pulse",
        "alsa",
        "audio device",
        "vo/gpu",
        "display",
        "wayland",
    ]) {
        "device"
    } else if has(&["decoder", "codec", "demux", "corrupt"]) {
        "decoder"
    } else if has(&["403", "expired", "cdn", "googlevideo"]) {
        "cdn"
    } else if has(&[
        "proxy",
        "network",
        "timeout",
        "timed out",
        "connection",
        "dns",
    ]) {
        "network"
    } else {
        "unknown"
    }
}
async fn tool(name: &'static str, program: &str, flag: &str, cancel: CancellationToken) -> Tool {
    let Some(path) = paths::executable(program) else {
        return Tool {
            name,
            state: "missing",
            version: None,
        };
    };
    let mut spec = ProcessSpec::new(path.to_string_lossy(), vec![flag.into()], 3);
    spec.max_output_bytes = 32 * 1024;
    match process::run(spec, cancel, |_| {}).await {
        Ok(result) if result.success => {
            // Retain a numeric version only. Arbitrary tool stdout may contain
            // home directories, URLs or credentials and is never exported.
            let version = result
                .lines
                .iter()
                .take(2)
                .flat_map(|line| line.split_whitespace())
                .find_map(|word| {
                    let word = word.trim_start_matches('v');
                    (word.len() <= 40
                        && word.bytes().next().is_some_and(|b| b.is_ascii_digit())
                        && word
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b)))
                    .then(|| word.to_owned())
                });
            Tool {
                name,
                state: "available",
                version,
            }
        }
        _ => Tool {
            name,
            state: "failed",
            version: None,
        },
    }
}
impl Support {
    pub fn with_active_backend(mut self, backend: &str) -> Self {
        self.active_backend = [
            "native Rust",
            "yt-dlp",
            "yt-dlp fallback",
            "direct stream",
            "local",
        ]
        .into_iter()
        .find(|known| *known == backend);
        self
    }
    pub async fn collect(
        config: &Config,
        version: &str,
        revision: &str,
        error: &str,
        cancel: CancellationToken,
    ) -> Self {
        let (mpv, ffmpeg, ffprobe, external, runtime) = tokio::join!(
            tool("mpv", &config.mpv, "--version", cancel.clone()),
            tool("FFmpeg", &config.ffmpeg, "-version", cancel.clone()),
            tool("ffprobe", &config.ffprobe, "-version", cancel.clone()),
            tool(
                "yt-dlp (optional playback fallback)",
                &config.yt_dlp,
                "--version",
                cancel.clone()
            ),
            native_runtime::select(config, cancel.clone()),
        );
        let runtime = match runtime {
            Ok(Some(path)) => {
                tool(
                    "native JavaScript",
                    &path.to_string_lossy(),
                    "--version",
                    cancel,
                )
                .await
            }
            Ok(None) => Tool {
                name: "native JavaScript",
                state: "missing",
                version: None,
            },
            Err(_) => Tool {
                name: "native JavaScript",
                state: "unsupported or failed",
                version: None,
            },
        };
        Self {
            schema: 1,
            version: version.into(),
            revision: revision.into(),
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            playback_backend: config.playback_backend.clone(),
            active_backend: None,
            download_backend: config.download_backend.clone(),
            proxy_scheme: config
                .proxy
                .as_deref()
                .and_then(|proxy| url::Url::parse(proxy).ok())
                .map(|url| url.scheme().to_owned())
                .unwrap_or_else(|| "system/default".into()),
            failure_phase: phase(error),
            failure_phase_source: "message classification",
            tools: vec![mpv, ffmpeg, ffprobe, external, runtime],
        }
    }
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("ytui {} · {}", self.version, self.revision),
            format!(
                "Playback: {} · Downloads: {} · Proxy: {}",
                self.playback_backend, self.download_backend, self.proxy_scheme
            ),
            format!(
                "Active backend: {} · inferred failure phase: {}",
                self.active_backend.unwrap_or("not active"),
                self.failure_phase
            ),
            String::new(),
        ];
        lines.extend(self.tools.iter().map(|tool| {
            format!(
                "{}: {} {}",
                tool.name,
                tool.state,
                tool.version.as_deref().unwrap_or("")
            )
        }));
        lines.extend([String::new(), "Tools must be installed explicitly using your package manager.".into(), "Native playback requires installed Node.js 22+ or pinned Deno 2.9.7.".into(), "FFmpeg/ffprobe handle conversion; mpv handles audio/video output.".into(), "mpv requires HTTP(S) proxy or OS VPN routing; SOCKS supports browsing/downloads only.".into(), "Settings → Tools changes apply to the next playback/download.".into(), "Support export includes this allowlist only; no raw errors, URLs, paths or session data.".into()]);
        lines
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn support_export_excludes_sessions_paths_urls_and_raw_errors() {
        let config = Config {
            proxy: Some("http://secret-user:secret-pass@127.0.0.1:9".into()),
            mpv: "/secret/home/missing".into(),
            ffmpeg: "/secret/home/missing".into(),
            ffprobe: "/secret/home/missing".into(),
            yt_dlp: "/secret/home/missing".into(),
            native_js_runtime: Some("/secret/home/missing".into()),
            ..Default::default()
        };
        let report = Support::collect(
            &config,
            "fixture",
            "0123456789",
            "CDN 403 https://signed.example/?sig=SECRET Cookie: token",
            CancellationToken::new(),
        )
        .await;
        let encoded = serde_json::to_string(&report).unwrap();
        assert_eq!(report.failure_phase, "cdn");
        for secret in [
            "secret-user",
            "secret-pass",
            "/secret/home",
            "signed.example",
            "SECRET",
            "Cookie",
        ] {
            assert!(!encoded.contains(secret));
        }
        assert_eq!(report.tools[0].state, "missing");
    }
}
