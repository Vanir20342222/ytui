//! Loopback/scripted recovery regressions; no public provider or session access.
use super::*;
use std::os::unix::fs::PermissionsExt;

fn paths(root: &Path) -> Paths {
    Paths {
        config_override: None,
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        xdg_state: root.join("xdg"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    }
}
fn actor(root: &Path, live: bool) -> Actor {
    Actor {
        config: Config {
            playback_sponsorblock: "off".into(),
            ..Default::default()
        },
        paths: paths(root),
        session: None,
        full: None,
        preview: None,
        state: PlayerStatus {
            backend: "native Rust".into(),
            volume: 23.0,
            speed: 1.25,
            position: 42.0,
            duration: if live { None } else { Some(100.0) },
            ..Default::default()
        },
        playing: Some((
            Media {
                url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
                live,
                ..Default::default()
            },
            live,
            false,
        )),
        reconnect_count: 0,
        reconnect_at: None,
        resume_after_preview: false,
        sponsor_task: None,
        ignore_segments_until: 0.0,
        streams: Arc::new(crate::cache::ResponseCache::new(16, 512 * 1024)),
        preparation: None,
        prepare_backoff: None,
    }
}
async fn finished(actor: &mut Actor, success: bool, message: &str) {
    let message = message.to_owned();
    let task = tokio::spawn(async move {
        Ok(ProcessResult {
            success,
            lines: if success {
                vec!["YTUI_END_FILE:eof".into()]
            } else {
                vec![]
            },
            stderr: message,
        })
    });
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }
    actor.full = Some(Active {
        progress_at: tokio::time::Instant::now(),
        last_position: None,
        cancel: CancellationToken::new(),
        task,
        socket: actor.paths.state.join("no-ipc"),
        backend: watch::channel("native Rust".into()).1,
        catalog: watch::channel(Default::default()).1,
        _dir: tempfile::tempdir().unwrap(),
        _cookies: None,
    });
}
#[tokio::test]
async fn rate_limited_native_streams_are_not_retried_after_one_second() {
    let temp = tempfile::tempdir().unwrap();
    for live in [false, true] {
        let mut actor = actor(temp.path(), live);
        finished(
            &mut actor,
            false,
            "[ffmpeg] HTTP error 429 Too Many Requests",
        )
        .await;
        let event = actor.tick().await;
        assert!(
            matches!(event, Some(PlayerEvent::Error(_))),
            "A rate limit must surface for cooldown"
        );
        assert!(actor.reconnect_at.is_none());
        assert!(!actor.state.buffering);
    }
    for message in [
        "HTTP error 429",
        "HTTP 429",
        "Server returned 429",
        "Too Many Requests",
        "YouTube rate limit",
        "HTTP status client error (429)",
        "status code 429",
    ] {
        assert!(!native_stream_retryable(message), "{message}");
        assert!(!crate::native::fallback_eligible(&format!(
            "No eligible media formats. {message}"
        )));
    }
    assert!(crate::native::fallback_eligible(
        "No eligible media formats; signature URL challenges"
    ));
}
#[tokio::test]
async fn exhausted_live_reconnects_report_error_even_after_clean_eof() {
    let temp = tempfile::tempdir().unwrap();
    for success in [false, true] {
        let mut actor = actor(temp.path(), true);
        actor.reconnect_count = 3;
        finished(&mut actor, success, "Connection reset by peer").await;
        let event = actor.tick().await;
        assert!(
            matches!(event, Some(PlayerEvent::Error(_))),
            "Live disconnect became Ended or silent exit"
        );
        assert!(actor.reconnect_at.is_none());
        assert!(!actor.state.buffering && !actor.state.active);
    }
}
#[tokio::test]
async fn controls_during_reconnect_update_the_preferences_for_the_next_launch() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), true);
    actor.reconnect_at = Some(tokio::time::Instant::now() + Duration::from_secs(30));
    actor.handle(PlayerCommand::Volume(-5.0)).await;
    actor.handle(PlayerCommand::Mute).await;
    actor.handle(PlayerCommand::Speed(0.25)).await;
    actor.handle(PlayerCommand::TogglePause).await;
    assert_eq!(actor.state.volume, 18.0);
    assert!(actor.state.muted && actor.state.paused);
    assert_eq!(actor.state.speed, 1.5);
    actor.handle(PlayerCommand::SetPaused(false)).await;
    assert!(!actor.state.paused);
    assert!(actor.reconnect_at.is_some());
}
#[tokio::test]
async fn reconnect_retains_the_explicit_external_backend_and_cached_controls() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), true);
    let extracted = temp.path().join("extracted");
    let args = temp.path().join("args");
    let extractor = temp.path().join("yt-dlp");
    let mpv = temp.path().join("mpv");
    std::fs::write(&extractor,format!("#!/bin/sh\nprintf yes > '{}'\nprintf '%s\\n' '{{\"url\":\"https://media.test/live\"}}'\n",extracted.display())).unwrap();
    std::fs::write(
        &mpv,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nsleep 30\n",
            args.display()
        ),
    )
    .unwrap();
    for path in [&extractor, &mpv] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    actor.config.yt_dlp = extractor.to_string_lossy().into();
    actor.config.mpv = mpv.to_string_lossy().into();
    actor.config.native_js_runtime = Some("/native-must-not-run".into());
    actor.playing.as_mut().unwrap().2 = true;
    actor.state.paused = true;
    actor.state.muted = true;
    actor.reconnect_at = Some(tokio::time::Instant::now());
    let _ = actor.tick().await;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&args)
                && text.ends_with("https://media.test/live\n")
            {
                break text;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    actor.stop().await;
    let recorded = result.expect("Explicit yt-dlp reconnect switched back to native");
    assert!(extracted.exists());
    assert!(recorded.contains("--volume=23\n"));
    assert!(recorded.contains("--pause=yes\n") && recorded.contains("--mute=yes\n"));
    assert_eq!(actor.config.playback_backend, "native");
}

#[tokio::test]
async fn live_reconnect_budget_is_bounded_and_never_advances_the_queue() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), true);
    for attempt in 0..=3 {
        actor.reconnect_at = None;
        finished(&mut actor, true, "").await;
        let event = actor.tick().await;
        if attempt < 3 {
            assert!(event.is_none());
            assert!(actor.reconnect_at.is_some());
            assert_eq!(actor.reconnect_count, attempt + 1);
        } else {
            assert!(matches!(event, Some(PlayerEvent::Error(_))));
            assert!(actor.reconnect_at.is_none());
        }
    }
}
#[tokio::test]
async fn stop_and_account_handoff_cancel_pending_reconnects() {
    let temp = tempfile::tempdir().unwrap();
    for command in [PlayerCommand::Stop, PlayerCommand::Session(None)] {
        let mut actor = actor(temp.path(), true);
        actor.reconnect_at = Some(tokio::time::Instant::now());
        actor.handle(command).await;
        assert!(actor.reconnect_at.is_none() && actor.playing.is_none());
        assert!(actor.tick().await.is_none());
        assert!(actor.full.is_none());
    }
}
#[tokio::test]
async fn pending_controls_are_finite_clamped_and_retained_as_preferences() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), true);
    actor.reconnect_at = Some(tokio::time::Instant::now() + Duration::from_secs(30));
    actor.handle(PlayerCommand::Volume(-1000.0)).await;
    actor.handle(PlayerCommand::Speed(1000.0)).await;
    actor.handle(PlayerCommand::Volume(f64::NAN)).await;
    actor.handle(PlayerCommand::Speed(f64::INFINITY)).await;
    assert_eq!(actor.preferences().volume, 0.0);
    assert_eq!(actor.preferences().speed, 4.0);
    actor.handle(PlayerCommand::SetVolume(40.0)).await;
    actor.handle(PlayerCommand::SetRate(1.75)).await;
    assert_eq!(actor.preferences().volume, 40.0);
    assert_eq!(actor.preferences().speed, 1.75);
}

#[tokio::test]
#[ignore = "requires actual mpv/FFmpeg; synthetic loopback live EOF and reconnect only"]
async fn real_mpv_stream_reconnect_retains_controls_and_never_emits_queue_completion() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let short = root.join("short.wav");
    let long = root.join("long.wav");
    for (path, seconds) in [(&short, "0.6"), (&long, "10")] {
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-t",
                seconds,
            ])
            .arg(path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let launches = root.join("launches");
    let args = root.join("last-args");
    let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
    let wrapper = root.join("mpv-null");
    std::fs::write(&wrapper,format!("#!/bin/sh\nprintf x >> '{}'\nprintf '%s\\n' \"$@\" > '{}'\nexec '{}' --vo=null --ao=null \"$@\"\n",launches.display(),args.display(),mpv.replace('\'',"'\\''"))).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let short = std::fs::read(short).unwrap();
    let long = std::fs::read(long).unwrap();
    let observed = launches.clone();
    let (base, server) = crate::test_http::serve(move |_| {
        let second = std::fs::read(&observed).is_ok_and(|bytes| bytes.len() >= 2);
        (
            200,
            "Content-Type: audio/wav\r\nAccept-Ranges: none\r\n".into(),
            if second { long.clone() } else { short.clone() },
        )
    })
    .await;
    let paths = paths(root);
    paths.create().unwrap();
    let mut player = Player::spawn(
        Config {
            mpv: wrapper.to_string_lossy().into(),
            playback_sponsorblock: "off".into(),
            ..Default::default()
        },
        paths,
        None,
    );
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        player.commands.send(PlayerCommand::SetVolume(23.0)).await?;
        player
            .commands
            .send(PlayerCommand::Play {
                media: Media {
                    title: "Live reconnect fixture".into(),
                    url: format!("{base}/live.wav"),
                    kind: ytui_core::MediaKind::Station,
                    live: true,
                    ..Default::default()
                },
                audio: true,
            })
            .await?;
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.buffering
                && !state.active
                && state
                    .error
                    .as_deref()
                    .is_some_and(|s| s.contains("reconnecting"))
            {
                break;
            }
            if let Ok(PlayerEvent::Error(error)) = player.events.try_recv() {
                anyhow::bail!(error);
            }
            player.status.changed().await?;
        }
        for command in [
            PlayerCommand::Volume(7.0),
            PlayerCommand::Mute,
            PlayerCommand::Speed(0.25),
            PlayerCommand::TogglePause,
        ] {
            player.commands.send(command).await?;
        }
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.active && state.paused && state.duration.is_some_and(|d| d > 5.0) {
                anyhow::ensure!(
                    state.volume == 30.0 && state.muted && state.speed == 1.25,
                    "Reconnect lost cached controls: {state:?}"
                );
                break;
            }
            if let Ok(event) = player.events.try_recv() {
                anyhow::bail!("Unexpected recovery event: {event:?}");
            }
            player.status.changed().await?;
        }
        player
            .commands
            .send(PlayerCommand::SetPaused(false))
            .await?;
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.active && !state.paused && state.position >= 0.25 {
                break;
            }
            if let Ok(event) = player.events.try_recv() {
                anyhow::bail!("Unexpected recovery event: {event:?}");
            }
            player.status.changed().await?;
        }
        let recorded = std::fs::read_to_string(&args)?;
        anyhow::ensure!(
            recorded.contains("--pause=yes\n")
                && recorded.contains("--volume=30\n")
                && recorded.contains("--mute=yes\n")
                && recorded.contains("--speed=1.25\n"),
            "Replacement mpv arguments lost controls"
        );
        anyhow::ensure!(
            std::fs::read(&launches)?.len() == 2,
            "Unexpected reconnect count"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await;
    player.close().await;
    server.abort();
    result.expect("Live reconnect fixture timed out").unwrap();
}

#[tokio::test]
async fn prepared_extraction_is_promoted_without_a_second_resolve_and_stop_cancels_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), false);
    let mpv = temp.path().join("mpv-prepared");
    crate::paths::atomic_write(&mpv, b"#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&mpv, std::fs::Permissions::from_mode(0o700)).unwrap();
    actor.config.mpv = mpv.display().to_string();
    actor.config.native_js_runtime = Some("/must-not-run-a-second-extraction".into());
    let media = actor.playing.as_ref().unwrap().0.clone();
    let options = actor.config.download.clone();
    let key = crate::playback_cache::key(&actor.config, &media, &options).unwrap();
    let release = Arc::new(tokio::sync::Notify::new());
    let ready = release.clone();
    let preparation = crate::playback_cache::Preparation::spawn(key, move |_| async move {
        ready.notified().await;
        Ok(crate::native::Resolved {
            url: "https://cdn.invalid/prepared".into(),
            audio_url: None,
            headers: vec![],
            catalog: Default::default(),
        })
    });
    let cancel = preparation.cancel.clone();
    actor.preparation = Some(preparation);
    actor.play(media, false, false, None, false).await;
    assert!(actor.preparation.is_none());
    assert!(actor.full.is_some());
    assert!(!cancel.is_cancelled());
    release.notify_one();
    let full = actor.full.as_mut().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while full.backend.borrow_and_update().is_empty() {
            full.backend.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(&*full.backend.borrow(), "native Rust");
    assert!(!cancel.is_cancelled());
    actor.stop().await;
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn speculative_rate_limits_cool_down_without_playback_errors_or_external_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), false);
    actor.preparation = Some(crate::playback_cache::Preparation::spawn(
        "fixture".into(),
        |_| async { anyhow::bail!("HTTP Error 429: Too Many Requests") },
    ));
    while !actor.preparation.as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    actor.tick().await;
    assert!(actor.state.error.is_none());
    assert!(actor.prepare_backoff.is_some());
    actor
        .handle(PlayerCommand::Prepare {
            media: actor.playing.as_ref().unwrap().0.clone(),
            audio: false,
        })
        .await;
    assert!(actor.preparation.is_none());
}

#[tokio::test]
async fn session_handoff_and_configuration_cancel_preparation_before_replacing_cache() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), false);
    for command in [
        PlayerCommand::Session(None),
        PlayerCommand::Configure(Box::default()),
    ] {
        let preparation =
            crate::playback_cache::Preparation::spawn("fixture".into(), |cancel| async move {
                cancel.cancelled().await;
                anyhow::bail!("cancelled")
            });
        let cancel = preparation.cancel.clone();
        actor.preparation = Some(preparation);
        actor.handle(command).await;
        assert!(cancel.is_cancelled());
        assert!(actor.preparation.is_none());
    }
}

#[tokio::test]
async fn an_expired_account_fails_before_extraction_instead_of_becoming_a_format_error() {
    let temp = tempfile::tempdir().unwrap();
    let mut actor = actor(temp.path(), false);
    actor.session = Some(Arc::new(Session {
        cookies: vec![crate::auth::Cookie {
            domain: ".youtube.com".into(),
            path: "/".into(),
            secure: true,
            expires: Some(1),
            name: "SAPISID".into(),
            value: "synthetic-expired".into(),
            http_only: false,
        }],
        account: "Synthetic account".into(),
        imported_at: 0,
    }));
    actor.config.native_js_runtime = Some("/must-not-probe-any-runtime".into());
    let media = actor.playing.as_ref().unwrap().0.clone();
    actor.play(media, false, false, None, false).await;
    assert!(actor.full.is_none());
    assert!(
        actor
            .state
            .error
            .as_deref()
            .unwrap()
            .contains("session expired")
    );
    assert!(!temp.path().join("state/auth/transient").exists());
}
