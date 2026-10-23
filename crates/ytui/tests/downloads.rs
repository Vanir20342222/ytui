//! End-to-end transfer fixtures: no YouTube account, Internet, Python or audio device.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
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
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("Executable:"));
    for args in [
        vec!["doctor", "--network"],
        vec!["browse", "home"],
        vec![
            "playback-check",
            "https://www.youtube.com/watch?v=abcdefghijk",
        ],
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
#[cfg(target_os = "linux")]
#[test]
fn native_worker_survives_replacing_the_running_executable() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let running = temp.path().join("ytui-running");
    if std::fs::hard_link(env!("CARGO_BIN_EXE_ytui"), &running).is_err() {
        std::fs::copy(env!("CARGO_BIN_EXE_ytui"), &running).unwrap();
    }
    let ready = temp.path().join("runtime-ready");
    let release = temp.path().join("runtime-release");
    let runtime = temp.path().join("node");
    std::fs::write(&runtime, format!("#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nprintf 'v26.8.1\\n'\n", ready.display(), release.display())).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    let proxy = Server::with_status(b"Synthetic proxy rejection".to_vec(), 403);
    let config = Config {
        auth_store: "file".into(),
        theme: "ytui-dark".into(),
        playback_backend: "native".into(),
        native_js_runtime: Some(runtime),
        proxy: Some(
            url::Url::parse(&proxy.url)
                .unwrap()
                .origin()
                .ascii_serialization(),
        ),
        ..Default::default()
    };
    let path = temp.path().join("config.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let child = Command::new(&running)
        .arg("--config")
        .arg(path)
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args([
            "playback-check",
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        ])
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_STATE_HOME", temp.path().join("xdg"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while !ready.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "runtime barrier never opened"
        );
        thread::sleep(Duration::from_millis(5));
    }
    // An atomic upgrade unlinks the executable's original directory entry.
    std::fs::remove_file(&running).unwrap();
    std::fs::write(&release, b"continue").unwrap();
    let output = child.wait_with_output().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        !error.contains("(deleted)"),
        "running-binary replacement broke native worker launch: {error}"
    );
    assert!(
        error.to_lowercase().contains("proxy") || error.to_lowercase().contains("tunnel"),
        "worker must reach the rejecting fixture proxy: {error}"
    );
}
#[test]
fn doctor_network_returns_failure_when_its_public_probes_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    // A local rejecting HTTP proxy prevents all actual external provider access.
    let proxy = Server::with_status(b"Fixture proxy rejection".to_vec(), 403);
    let config = Config {
        theme: "ytui-dark".into(),
        auth_store: "file".into(),
        proxy: Some(
            url::Url::parse(&proxy.url)
                .unwrap()
                .origin()
                .ascii_serialization(),
        ),
        ..Default::default()
    };
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .arg("--config")
        .arg(config_file)
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args(["doctor", "--network"])
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(output.contains("YouTube public search: FAILED"), "{output}");
    assert!(output.contains("Music public search: FAILED"));
    assert!(String::from_utf8_lossy(&result.stderr).contains("public network probe(s) failed"));
}
#[test]
fn native_playback_never_invokes_external_extraction_even_for_legacy_auto_configs() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let proxy = Server::with_status(b"Fixture rejection".to_vec(), 403);
    let marker = temp.path().join("external-was-used");
    let extractor = temp.path().join("yt-dlp");
    std::fs::write(
        &extractor,
        format!("#!/bin/sh\nprintf x > '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&extractor, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Deliberately deserialize a config predating playback_backend.
    let mut config: Config =
        toml::from_str("download_backend='auto'\nauth_store='file'\ntheme='ytui-dark'").unwrap();
    config.proxy = Some(
        url::Url::parse(&proxy.url)
            .unwrap()
            .origin()
            .ascii_serialization(),
    );
    config.yt_dlp = extractor.display().to_string();
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .arg("--config")
        .arg(config_file)
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args([
            "playback-check",
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        ])
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Native playback"));
    assert!(
        !marker.exists(),
        "native playback silently started external yt-dlp"
    );
}
impl Server {
    fn start() -> Self {
        Self::with_payload(b"synthetic-media-fixture".to_vec())
    }
    fn with_payload(payload: Vec<u8>) -> Self {
        Self::with_status(payload, 200)
    }
    fn with_status(payload: Vec<u8>, status: u16) -> Self {
        Self::with_handler(move |_| (status, "video/mp4", payload.clone()))
    }
    fn with_handler(
        handler: impl Fn(&str) -> (u16, &'static str, Vec<u8>) + Send + 'static,
    ) -> Self {
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
                        let count = stream.read(&mut request).unwrap_or(0);
                        let request = String::from_utf8_lossy(&request[..count]);
                        let target = request
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or("/");
                        let (status, mime, payload) = handler(target);
                        let headers = format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n",
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

#[cfg(unix)]
#[test]
fn automatic_fallback_runs_once_for_a_native_limitation_but_strict_native_stays_native() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let final_file = temp.path().join("fallback.mp4");
    let marker = temp.path().join("fallback-called");
    let script = temp.path().join("yt-dlp-fixture");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\nprintf media > '{}'\nprintf '%s\\n' 'YTUI_OUTPUT:{}'\n",
            marker.display(),
            final_file.display(),
            serde_json::to_string(&final_file.to_string_lossy()).unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = Config {
        auth_store: "file".into(),
        yt_dlp: script.display().to_string(),
        max_retries: 0,
        ..Default::default()
    };
    config.download.metadata = false;
    config.download.sponsorblock = vec!["sponsor".into()];
    config.download.sponsorblock_remove = true;
    config.download.subtitles = vec!["en".into()];
    let config_path = temp.path().join("config.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let run = |backend: &str| {
        Command::new(env!("CARGO_BIN_EXE_ytui"))
            .args([
                "--config",
                config_path.to_str().unwrap(),
                "--state-dir",
                temp.path().join("state").to_str().unwrap(),
                "download",
                "https://www.youtube.com/watch?v=BaW_jenozKc",
                "--directory",
                temp.path().join("media").to_str().unwrap(),
                "--backend",
                backend,
            ])
            .output()
            .unwrap()
    };
    let strict = run("native");
    assert!(!strict.status.success());
    assert!(!marker.exists());
    let automatic = run("auto");
    assert!(
        automatic.status.success(),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );
    assert_eq!(std::fs::read(&marker).unwrap(), b"x");
    assert_eq!(std::fs::read(&final_file).unwrap(), b"media");
}

#[cfg(unix)]
#[test]
fn playlist_downloads_keep_all_outputs_source_and_explicit_item_selection() {
    use std::os::unix::fs::PermissionsExt;
    for (case, url, audio, extra, expected) in [
        (
            "music",
            "https://music.youtube.com/playlist?list=PLsynthetic",
            true,
            vec![],
            3,
        ),
        (
            "video",
            "https://www.youtube.com/playlist?list=PLsynthetic",
            false,
            vec![],
            3,
        ),
        (
            "watch-playlist",
            "https://www.youtube.com/watch?v=synthetic00&list=PLsynthetic",
            true,
            vec!["--playlist"],
            3,
        ),
        (
            "watch-single",
            "https://www.youtube.com/watch?v=synthetic00&list=PLsynthetic",
            true,
            vec![],
            1,
        ),
        (
            "selected",
            "https://music.youtube.com/playlist?list=PLsynthetic",
            true,
            vec!["--items", "2,3"],
            2,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("arguments");
        let media = temp.path().join("media");
        std::fs::create_dir(&media).unwrap();
        let script = temp.path().join("yt-dlp");
        std::fs::write(&script,format!(r#"#!/bin/sh
printf '%s\n' "$@" > '{}'
entries='1'
while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes-playlist) entries='1 2 3' ;;
    --playlist-items) shift; [ "$1" = '2,3' ] && entries='2 3' ;;
  esac
  shift
done
for entry in $entries; do
  final='{}/track-'$entry'.media'
  printf media > "$final"
  printf '%s\n' 'YTUI_META:{{"id":"track-'$entry'","title":"Track '$entry'","webpage_url":"https://www.youtube.com/watch?v=track'$entry'"}}'
  printf 'YTUI_OUTPUT:"%s"\n' "$final"
done
"#,record.display(),media.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = Config {
            auth_store: "file".into(),
            yt_dlp: script.display().to_string(),
            max_retries: 0,
            download_backend: if case == "watch-single" {
                "yt-dlp"
            } else {
                "auto"
            }
            .into(),
            ..Default::default()
        };
        let config_file = temp.path().join("config.toml");
        std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
        let state = temp.path().join("state");
        let mut command = Command::new(env!("CARGO_BIN_EXE_ytui"));
        command
            .args([
                "--config",
                config_file.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "download",
                url,
                "--backend",
                if case == "watch-single" {
                    "yt-dlp"
                } else {
                    "auto"
                },
                "--directory",
                media.to_str().unwrap(),
            ])
            .args(extra);
        if audio {
            command.arg("--audio");
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout)
                .matches("Saved ")
                .count(),
            expected,
            "{case}"
        );
        let args = std::fs::read_to_string(record).unwrap();
        assert!(
            args.contains(if expected == 1 {
                "--no-playlist\n"
            } else {
                "--yes-playlist\n"
            }),
            "{case}: {args}"
        );
        let snapshot = tokio::runtime::Runtime::new().unwrap().block_on(async {
            let store = ytui_services::store::Store::open(&state).unwrap();
            let snapshot = store.load().await.unwrap();
            store.close().await.unwrap();
            snapshot
        });
        let job = &snapshot.downloads.jobs[0];
        assert_eq!(job.outputs.len(), expected, "{case}");
        assert!(job.outputs.iter().all(|path| path.is_file()));
        assert_eq!(job.state, ytui_core::JobState::Done);
        if expected > 1 {
            assert_eq!(job.media.url, url, "{case}: entry replaced playlist source");
        }
        if case == "selected" {
            assert_eq!(job.options.playlist_items.as_deref(), Some("2,3"));
        }
    }
}
#[cfg(unix)]
#[test]
fn failed_playlist_retains_completed_outputs_and_strict_native_reports_limit() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let final_file = temp.path().join("first.m4a");
    let script = temp.path().join("yt-dlp");
    std::fs::write(&script,format!("#!/bin/sh\nprintf media > '{}'\nprintf '%s\\n' 'YTUI_OUTPUT:{}'\necho 'Playlist item 2 is unavailable' >&2\nexit 1\n",final_file.display(),serde_json::to_string(&final_file.display().to_string()).unwrap())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = Config {
        auth_store: "file".into(),
        yt_dlp: script.display().to_string(),
        max_retries: 0,
        ..Default::default()
    };
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let source = "https://music.youtube.com/playlist?list=PLsynthetic";
    let run = |backend: &str, state: &Path| {
        Command::new(env!("CARGO_BIN_EXE_ytui"))
            .args([
                "--config",
                config_file.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "download",
                source,
                "--audio",
                "--backend",
                backend,
                "--directory",
                temp.path().join("media").to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let strict = run("native", &temp.path().join("strict-state"));
    assert!(!strict.status.success());
    assert!(
        !final_file.exists(),
        "strict native unexpectedly launched fallback"
    );
    assert!(
        String::from_utf8_lossy(&strict.stderr)
            .contains("Native YouTube playlists require yt-dlp fallback")
    );
    let state = temp.path().join("state");
    let failed = run("auto", &state);
    assert!(!failed.status.success());
    assert!(!String::from_utf8_lossy(&failed.stdout).contains("Saved "));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("item 2 is unavailable"));
    let snapshot = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = ytui_services::store::Store::open(&state).unwrap();
        let snapshot = store.load().await.unwrap();
        store.close().await.unwrap();
        snapshot
    });
    let job = &snapshot.downloads.jobs[0];
    assert_eq!(job.state, ytui_core::JobState::Error);
    assert_eq!(job.media.url, source);
    assert_eq!(job.outputs, vec![final_file]);
}

fn assert_real_audio_conversion(format: &str, codec: &str) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mp4");
    let generated = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
            "-t",
            "0.8",
            source.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let original = std::fs::read(&source).unwrap();
    let server = Server::with_payload(original.clone());
    let mut config = Config {
        auth_store: "file".into(),
        yt_dlp: "/must/not/run/yt-dlp".into(),
        max_retries: 0,
        ..Default::default()
    };
    config.download.audio_container = format.into();
    config.download.metadata = false;
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .args([
            "--config",
            config_file.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "download",
            &server.url,
            "--audio",
            "--backend",
            "native",
            "--directory",
            temp.path().join("music").to_str().unwrap(),
        ])
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = std::fs::read_dir(temp.path().join("music"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].extension().unwrap(), format);
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            files[0].to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(probe.status.success());
    let data: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(data["streams"][0]["codec_name"], codec);
    assert_eq!(data["streams"][0]["codec_type"], "audio");
    assert!(
        data["format"]["duration"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap()
            > 0.5
    );
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
#[ignore = "requires real FFmpeg and ffprobe; generated audio over loopback only"]
fn real_ffmpeg_native_audio_download_produces_a_decodable_mp3() {
    assert_real_audio_conversion("mp3", "mp3");
}
#[test]
#[ignore = "requires real FFmpeg and ffprobe; generated audio over loopback only"]
fn real_ffmpeg_native_audio_formats_m4a_opus_flac_are_decodable() {
    for (format, codec) in [("m4a", "aac"), ("opus", "opus"), ("flac", "flac")] {
        assert_real_audio_conversion(format, codec);
    }
}

#[tokio::test]
#[ignore = "requires actual yt-dlp, FFmpeg and ffprobe; two HTML media entries served over loopback, no YouTube"]
async fn real_ytdlp_playlist_saves_two_distinct_decodable_audio_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mp4");
    let generated = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
            "-t",
            "2",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let bytes = std::fs::read(source).unwrap();
    let server = Server::with_handler(move |target| {
        match target {
        "/playlist.html" => (200, "text/html", b"<!doctype html><html><head><title>Playlist fixture</title></head><body><video src='/one.mp4'></video><video src='/two.mp4'></video></body></html>".to_vec()),
        "/one.mp4" | "/two.mp4" => (200, "video/mp4", bytes.clone()),
        _ => (404, "text/plain", b"Missing fixture".to_vec()),
    }
    });
    let url = server.url.replace("fixture.mp4", "playlist.html");
    let config = Config {
        theme: "ytui-dark".into(),
        auth_store: "file".into(),
        max_retries: 0,
        yt_dlp: std::env::var("YTUI_TEST_YT_DLP").unwrap_or_else(|_| "yt-dlp".into()),
        ..Default::default()
    };
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_ytui"))
        .arg("--config")
        .arg(config_file)
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args([
            "download",
            &url,
            "--playlist",
            "--audio",
            "--backend",
            "yt-dlp",
            "--now",
            "--directory",
        ])
        .arg(temp.path().join("output"))
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let store = ytui_services::store::Store::open(&temp.path().join("state")).unwrap();
    let snapshot = store.load().await.unwrap();
    store.close().await.unwrap();
    let job = &snapshot.downloads.jobs[0];
    assert_eq!(job.state, ytui_core::JobState::Done);
    assert_eq!(job.media.url, url);
    assert_eq!(
        job.outputs.len(),
        2,
        "real yt-dlp saved only the first playlist entry"
    );
    assert_ne!(job.outputs[0], job.outputs[1]);
    for path in &job.outputs {
        assert!(path.metadata().unwrap().len() > 0);
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_name:format=duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let json: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(json["streams"][0]["codec_name"], "mp3");
        assert!(
            json["format"]["duration"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                >= 1.9
        );
    }
}

#[test]
#[ignore = "requires actual mpv and FFmpeg; generated six-second video over loopback, no YouTube"]
fn real_mpv_playback_check_requires_actual_progress_and_reports_tool_failure() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mp4");
    let generated = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=64x64:r=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "6",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let server = Server::with_payload(std::fs::read(source).unwrap());
    let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
    let mpv = ytui_services::paths::executable(&mpv).expect("Install mpv or set YTUI_TEST_MPV");
    let wrapper = temp.path().join("mpv-headless");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' --vo=null --ao=null \"$@\"\n",
            mpv.to_string_lossy().replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = Config {
        mpv: wrapper.display().to_string(),
        auth_store: "file".into(),
        playback_sponsorblock: "off".into(),
        ..Default::default()
    };
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_ytui"))
            .arg("--config")
            .arg(&config_file)
            .arg("--state-dir")
            .arg(temp.path().join("state"))
            .args(["playback-check", &server.url, "--seconds", "2"])
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_STATE_HOME", temp.path().join("state"))
            .env("XDG_CACHE_HOME", temp.path().join("cache"))
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .output()
            .unwrap()
    };
    let result = check();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(value["position"].as_f64().unwrap() >= 2.0);
    assert!(value["duration"].as_f64().unwrap() >= 5.0);
    std::fs::write(
        &wrapper,
        "#!/bin/sh\necho 'Fixture decoder failure'\nexit 1\n",
    )
    .unwrap();
    let result = check();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Fixture decoder failure"));
}

#[cfg(unix)]
#[test]
fn rejected_native_http_stream_uses_one_fallback_and_destination_errors_do_not() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::with_status(b"Forbidden".to_vec(), 403);
    let record = temp.path().join("fallback-count");
    let final_file = temp.path().join("video.mp4");
    let script = temp.path().join("yt-dlp");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\nprintf media > '{}'\nprintf '%s\\n' 'YTUI_OUTPUT:{}'\n",
            record.display(),
            final_file.display(),
            serde_json::to_string(&final_file.display().to_string()).unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = Config {
        auth_store: "file".into(),
        yt_dlp: script.display().to_string(),
        max_retries: 0,
        ..Default::default()
    };
    let config_file = temp.path().join("config.toml");
    std::fs::write(&config_file, toml::to_string(&config).unwrap()).unwrap();
    let run = |backend: &str, destination: &Path| {
        Command::new(env!("CARGO_BIN_EXE_ytui"))
            .args([
                "--config",
                config_file.to_str().unwrap(),
                "--state-dir",
                temp.path().join("state").to_str().unwrap(),
                "download",
                &server.url,
                "--backend",
                backend,
                "--directory",
                destination.to_str().unwrap(),
            ])
            .env("NO_PROXY", "127.0.0.1,localhost")
            .output()
            .unwrap()
    };
    let strict = run("native", &temp.path().join("media"));
    assert!(!strict.status.success());
    assert!(!record.exists());
    let automatic = run("auto", &temp.path().join("media"));
    assert!(
        automatic.status.success(),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );
    assert_eq!(std::fs::read(&record).unwrap(), b"x");
    let blocked = temp.path().join("not-a-directory");
    std::fs::write(&blocked, b"existing").unwrap();
    assert!(!run("auto", &blocked).status.success());
    assert_eq!(
        std::fs::read(record).unwrap(),
        b"x",
        "local destination failure launched a fallback"
    );
}
