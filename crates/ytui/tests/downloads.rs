//! End-to-end transfer fixtures: no YouTube account, Internet, Python or audio device.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use ytui_core::Config;

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let task = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = [0u8; 4096];
                        let _ = stream.read(&mut request);
                        let payload = b"synthetic-media-fixture";
                        let headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = stream.write_all(headers.as_bytes());
                        let _ = stream.write_all(payload);
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            url: format!("http://{addr}/fixture.mp4"),
            stop,
            thread: Some(task),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
#[test]
fn native_default_transfers_without_python_and_reports_a_real_final_file() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::start();
    let mut config = Config {
        auth_store: "file".into(),
        yt_dlp: "/does/not/exist/yt-dlp".into(),
        ..Default::default()
    };
    config.download.metadata = false;
    config.download.thumbnail = false;
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .args([
            "--config",
            path.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "download",
            &server.url,
            "--directory",
            temp.path().join("media").to_str().unwrap(),
        ])
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Saved"));
    let files = std::fs::read_dir(temp.path().join("media"))
        .unwrap()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    assert_eq!(
        std::fs::read(files[0].path()).unwrap(),
        b"synthetic-media-fixture"
    );
}
#[cfg(unix)]
#[test]
fn explicit_fallback_requires_success_after_postprocessing() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-yt-dlp");
    let final_file = temp.path().join("converted.m4a");
    let script_text = format!(
        "#!/bin/sh\nprintf '%s' media > '{}'\nprintf '%s\\n' 'YTUI_PROGRESS:{{\"downloaded_bytes\":100,\"total_bytes\":100}}' 'YTUI_OUTPUT:{}'\necho 'postprocessing failed' >&2\nexit 1\n",
        final_file.display(),
        serde_json::to_string(final_file.to_str().unwrap()).unwrap()
    );
    std::fs::write(&script, script_text).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = Config {
        auth_store: "file".into(),
        yt_dlp: script.to_string_lossy().into(),
        ..Default::default()
    };
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .args([
            "--config",
            path.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "download",
            "https://example.test/video",
            "--backend",
            "yt-dlp",
            "--directory",
            temp.path().join("media").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("postprocessing failed"));
    assert!(final_file.is_file());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Saved"));
}

#[cfg(unix)]
#[test]
fn local_conversion_changes_extension_and_preserves_input() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mkv");
    std::fs::write(&source, b"original source").unwrap();
    let script = temp.path().join("ffmpeg");
    std::fs::write(
        &script,
        "#!/bin/sh\nfor final do :; done\nprintf '%s' converted > \"$final\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = Config {
        auth_store: "file".into(),
        ffmpeg: script.to_string_lossy().into(),
        ..Default::default()
    };
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .args([
            "--config",
            path.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "download",
            source.to_str().unwrap(),
            "--audio",
            "--directory",
            temp.path().join("media").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(source).unwrap(), b"original source");
    assert_eq!(
        std::fs::read(temp.path().join("media/source_converted.mp3")).unwrap(),
        b"converted"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("source_converted.mp3"));
}

#[cfg(unix)]
#[test]
fn hooks_receive_resolved_metadata_and_daily_window_requires_explicit_override() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("fake-extractor");
    let hook = temp.path().join("hook");
    let final_file = temp.path().join("final.mp4");
    let environment_file = temp.path().join("hook-env");
    // Fixed test scripts with path data passed by arguments/environment, never evaluated as shell source.
    std::fs::write(&script, "#!/bin/sh\nprintf '%s' media > \"$FIXTURE_OUTPUT\"\nprintf '%s\\n' 'YTUI_META:{\"id\":\"fixture\",\"title\":\"日本語 title\",\"webpage_url\":\"https://example.test/video\"}'\nprintf 'YTUI_OUTPUT:\"%s\"\\n' \"$FIXTURE_OUTPUT\"\n").unwrap();
    std::fs::write(&hook, "#!/bin/sh\nprintf '%s\\n' \"$YTUI_OUTPUT_PATH\" \"$YTUI_VIDEO_ID\" \"$YTUI_TITLE\" \"$YTUI_URL\" \"$YTUI_DOWNLOAD_MODE\" > \"$FIXTURE_ENV\"\n").unwrap();
    for path in [&script, &hook] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut config = Config {
        auth_store: "file".into(),
        yt_dlp: script.to_string_lossy().into(),
        ..Default::default()
    };
    config.download.post_download_hook = Some(hook);
    let minute = (ytui_services::desktop::local_minute() + 20) % 1440;
    let end = (minute + 1) % 1440;
    config.schedule.download_window = Some(format!(
        "{:02}:{:02}-{:02}:{:02}",
        minute / 60,
        minute % 60,
        end / 60,
        end % 60
    ));
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ytui"));
    command
        .args([
            "--config",
            path.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "download",
            "https://example.test/video",
            "--backend",
            "yt-dlp",
            "--directory",
            temp.path().join("media").to_str().unwrap(),
        ])
        .env("FIXTURE_OUTPUT", &final_file)
        .env("FIXTURE_ENV", &environment_file);
    let blocked = command.output().unwrap();
    assert!(!blocked.status.success());
    assert!(!final_file.exists());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("--now"));
    let output = command.arg("--now").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let environment = std::fs::read_to_string(environment_file).unwrap();
    assert!(environment.contains("fixture\n日本語 title\nhttps://example.test/video\nvideo\n"));
    assert!(environment.contains(final_file.to_str().unwrap()));
}
