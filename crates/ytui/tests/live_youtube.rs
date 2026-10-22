//! Explicit public YouTube smoke checks. No browser profile or account is imported.
use std::{path::Path, process::Stdio, time::Duration};
async fn run(args: &[&str]) -> (std::process::Output, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let config = root.join("config.toml");
    std::fs::write(
        &config,
        "theme='ytui-dark'\nauth_store='file'\nnotifications=false\n",
    )
    .unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ytui"));
    command
        .args([
            "--config",
            config.to_str().unwrap(),
            "--state-dir",
            root.join("state").to_str().unwrap(),
        ])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, name) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
    ] {
        command.env(key, root.join(name));
    }
    let result = tokio::time::timeout(Duration::from_secs(90), command.output())
        .await
        .expect("Live request exceeded 90 seconds")
        .unwrap();
    (result, temp)
}
fn page(output: std::process::Output) -> ytui_core::Page {
    assert!(
        output.status.success(),
        "{}",
        ytui_services::process::diagnostic(&String::from_utf8_lossy(&output.stderr))
    );
    serde_json::from_slice(&output.stdout).expect("CLI did not return a page")
}
#[tokio::test]
#[ignore = "requires live public YouTube access; no account or cookies"]
async fn live_youtube_public_search() {
    let (output, _temp) = run(&["browse", "search", "Rust programming"]).await;
    assert!(!page(output).items.is_empty());
}
#[tokio::test]
#[ignore = "requires live public YouTube Music access; no account or cookies"]
async fn live_music_public_search() {
    let (output, _temp) = run(&["browse", "--music", "search", "classical music"]).await;
    assert!(!page(output).items.is_empty());
}
#[tokio::test]
#[ignore = "requires live public YouTube player/details access; no account or cookies"]
async fn live_youtube_video_details() {
    let (output, _temp) = run(&["browse", "details", "BaW_jenozKc"]).await;
    let page = page(output);
    assert_eq!(page.items[0].id, "BaW_jenozKc");
    assert!(!page.items[0].title.is_empty());
}
#[tokio::test]
#[ignore = "downloads the small public yt-dlp test video with native Rust; requires YouTube and FFmpeg"]
async fn live_native_youtube_test_video_download() {
    let destination = tempfile::tempdir().unwrap();
    let (output, _temp) = run(&[
        "download",
        "https://www.youtube.com/watch?v=BaW_jenozKc",
        "--quality",
        "360",
        "--directory",
        destination.path().to_str().unwrap(),
        "--backend",
        "native",
        "--now",
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        ytui_services::process::diagnostic(&String::from_utf8_lossy(&output.stderr))
    );
    assert!(
        std::fs::read_dir(destination.path())
            .unwrap()
            .flatten()
            .any(|e| e.path().is_file()
                && e.metadata().is_ok_and(|m| m.len() > 0)
                && Path::new(&e.file_name())
                    .extension()
                    .is_some_and(|e| e == "mp4"))
    );
}
