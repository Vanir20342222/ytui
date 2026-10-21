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

#[test]
fn headless_setup_station_import_and_offline_guards_preserve_user_files() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("rust.toml");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ytui"))
            .arg("--config")
            .arg(&config_path)
            .arg("--state-dir")
            .arg(temp.path().join("state"))
            .arg("--offline")
            .args(args)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_STATE_HOME", temp.path().join("xdg-state"))
            .env("XDG_CACHE_HOME", temp.path().join("cache"))
            .output()
            .unwrap()
    };
    assert!(run(&["init"]).status.success());
    assert!(
        Command::new(env!("CARGO_BIN_EXE_ytui"))
            .current_dir(temp.path())
            .args(["--config", "relative.toml", "--offline", "init"])
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_STATE_HOME", temp.path().join("xdg-state"))
            .env("XDG_CACHE_HOME", temp.path().join("cache"))
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(temp.path().join("relative.toml").is_file());
    let original = std::fs::read(&config_path).unwrap();
    assert!(!run(&["init"]).status.success());
    assert_eq!(std::fs::read(&config_path).unwrap(), original);
    let mut config: Config = toml::from_str(std::str::from_utf8(&original).unwrap()).unwrap();
    config.auth_store = "file".into();
    config.theme = "ytui-dark".into();
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let source = temp.path().join("cliamp.toml");
    let stations = "[[stations]]\nname='Fixture radio'\nurl='https://example.test/live'\n[[stations]]\nname='Duplicate'\nstream='https://example.test/live'\n[[stations]]\nname='Unsupported'\nurl='file:///synthetic/local'\n";
    std::fs::write(&source, stations).unwrap();
    assert!(
        run(&["import-radio", source.to_str().unwrap()])
            .status
            .success()
    );
    assert!(
        run(&["import-radio", source.to_str().unwrap()])
            .status
            .success()
    );
    let saved: Config = toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(saved.custom_stations.len(), 1);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), stations);
    let doctor = run(&["doctor"]);
    assert!(doctor.status.success());
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("Browser stores were not opened"));
    for args in [
        vec!["browse", "home"],
        vec!["download", "https://www.youtube.com/watch?v=abcdefghijk"],
        vec!["auth", "import", "--browser", "brave-origin"],
    ] {
        let result = run(&args);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("restart without --offline"));
    }
}

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        Self::with_payload(b"synthetic-media-fixture".to_vec())
    }
    fn with_payload(payload: Vec<u8>) -> Self {
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
                        let headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = stream.write_all(headers.as_bytes());
                        let _ = stream.write_all(&payload);
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

#[test]
fn native_media_transfer_honors_a_rate_limit_and_cleans_staging() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::with_payload(vec![42; 96 * 1024]);
    let mut config = Config {
        auth_store: "file".into(),
        yt_dlp: "/does/not/exist".into(),
        ..Default::default()
    };
    config.download.metadata = false;
    config.download.bandwidth_kib = 64;
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let started = std::time::Instant::now();
    let result = Command::new(env!("CARGO_BIN_EXE_ytui"))
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
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(started.elapsed() >= Duration::from_millis(1400));
    let files = std::fs::read_dir(temp.path().join("media"))
        .unwrap()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path()).unwrap().len(), 96 * 1024);
}

#[cfg(unix)]
#[test]
fn cancelling_a_throttled_native_worker_is_prompt_and_preserves_pause() {
    use std::process::Stdio;
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let temp = tempfile::tempdir().unwrap();
    let server = Server::with_payload(vec![42; 96 * 1024]);
    let mut config = Config {
        auth_store: "file".into(),
        ..Default::default()
    };
    config.download.metadata = false;
    config.download.bandwidth_kib = 1;
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ytui"))
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ready = std::time::Instant::now();
    loop {
        let media = temp.path().join("media");
        let staging_ready = std::fs::read_dir(&media)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.path().join("0").is_dir());
        if staging_ready {
            break;
        }
        if ready.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("native worker did not reach media staging");
        }
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(200));
    assert_eq!(unsafe { kill(child.id() as i32, 2) }, 0);
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            panic!("throttled worker did not stop promptly");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    if temp.path().join("media").exists() {
        assert_eq!(
            std::fs::read_dir(temp.path().join("media"))
                .unwrap()
                .count(),
            0
        );
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = ytui_services::store::Store::open(&temp.path().join("state")).unwrap();
        let saved = store.load().await.unwrap();
        assert_eq!(saved.downloads.jobs[0].state, ytui_core::JobState::Paused);
        store.close().await.unwrap();
    });
}

#[cfg(unix)]
#[test]
fn failed_native_tagging_keeps_a_recoverable_source_and_publishes_no_final_file() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::start();
    let ffmpeg = temp.path().join("ffmpeg");
    std::fs::write(&ffmpeg, "#!/bin/sh\necho 'embedding failed' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = Config {
        auth_store: "file".into(),
        ffmpeg: ffmpeg.to_string_lossy().into(),
        ..Default::default()
    };
    let path = temp.path().join("rust.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let media = temp.path().join("media");
    let result = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .args([
            "--config",
            path.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "download",
            &server.url,
            "--directory",
            media.to_str().unwrap(),
        ])
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("Recoverable source kept"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let entries = std::fs::read_dir(media)
        .unwrap()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].path().is_dir());
    let sources = std::fs::read_dir(entries[0].path().join("0"))
        .unwrap()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 1);
    assert_eq!(
        std::fs::read(sources[0].path()).unwrap(),
        b"synthetic-media-fixture"
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("Saved"));
}
