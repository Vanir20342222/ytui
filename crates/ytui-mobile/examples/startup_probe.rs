//! Explicit anonymous network diagnostic. Measures resolution only, never claims a visible frame.
use anyhow::{Result, ensure};
use serde_json::json;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let raw = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!(
            "Usage: cargo run -p ytui-mobile --example startup_probe -- PUBLIC_YOUTUBE_URL"
        )
    })?;
    let url = url::Url::parse(&raw)?;
    ensure!(
        url.scheme() == "https" && ytui_core::youtube_host(url.host_str().unwrap_or_default()),
        "Use a public HTTPS YouTube video URL; this probe never imports an account"
    );
    let temporary = tempfile::tempdir()?;
    let engine = ytui_mobile::Engine::new(
        &temporary.path().join("files"),
        &temporary.path().join("cache"),
    )?;
    for label in ["cold", "cached"] {
        let start = Instant::now();
        match engine.request(json!({"op":"resolve","media":{"url":raw,"kind":"video"},"audio":false,"quality":"720"})).await {
            Ok(stream) => println!("{label}: resolve_ms={}, cache_hit={}, paired={}; visible frame NOT measured",
                start.elapsed().as_millis(), stream["resolution_cache_hit"] == true, stream["audio_url"].is_string()),
            Err(error) => {
                let text = format!("{error:#}").to_ascii_lowercase();
                let category = if text.contains("429") { "rate_limited" }
                    else if text.contains("blocked") { "provider_ip_block" }
                    else if text.contains("403") { "provider_forbidden" }
                    else if text.contains("no eligible media") { "native_formats_unavailable" }
                    else if text.contains("error sending request") { "network_transport" }
                    else if text.contains("timed out") || text.contains("timeout") { "network_timeout" }
                    else { "extraction_unavailable" };
                println!("{label}: resolve_ms={}, failure={category}; visible frame NOT measured", start.elapsed().as_millis());
                anyhow::bail!("Public playback remains unqualified; no retry storm or credentials used");
            }
        }
    }
    Ok(())
}
