use super::*;
#[tokio::test]
#[ignore = "requires actual mpv/FFmpeg; generated stereo tone, null output, no public media"]
async fn real_mpv_audio_meter_tracks_stereo_and_live_style_changes_without_restart() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let wav = root.join("stereo.wav");
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=12",
            "-af",
            "pan=stereo|c0=c0|c1=0.1*c0",
        ])
        .arg(&wav)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
    let wrapper = root.join("mpv-null");
    let launches = root.join("launches");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\nexec '{}' --ao=null --vo=null \"$@\"\n",
            launches.display(),
            mpv.replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = Paths {
        config_override: None,
        home: root.into(),
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        xdg_state: root.join("xdg"),
        legacy_data: root.join("legacy"),
    };
    paths.create().unwrap();
    let mut config = Config {
        mpv: wrapper.to_string_lossy().into(),
        music_player_style: "vu".into(),
        playback_sponsorblock: "off".into(),
        ..Default::default()
    };
    let mut player = Player::spawn(config.clone(), paths, None);
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        player
            .commands
            .send(PlayerCommand::Play {
                media: Media {
                    url: wav.to_string_lossy().into(),
                    kind: ytui_core::MediaKind::Song,
                    ..Default::default()
                },
                audio: true,
            })
            .await?;
        loop {
            let state = player.status.borrow_and_update().clone();
            if let Some(level) = state.audio_levels {
                anyhow::ensure!(
                    level.left > level.right + 0.2,
                    "Stereo analysis lost channel separation: {level:?}"
                );
                anyhow::ensure!(state.position > 0.0);
                break;
            }
            if let Some(e) = state.error {
                anyhow::bail!(e);
            }
            player.status.changed().await?;
        }
        player.commands.send(PlayerCommand::SetPaused(true)).await?;
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.paused {
                break;
            }
            player.status.changed().await?;
        }
        config.music_player_style = "orbit-fullscreen".into();
        player
            .commands
            .send(PlayerCommand::Configure(Box::new(config.clone())))
            .await?;
        config.music_player_style = "classic".into();
        player
            .commands
            .send(PlayerCommand::Configure(Box::new(config.clone())))
            .await?;
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.audio_levels.is_none() && state.paused && state.active {
                break;
            }
            player.status.changed().await?;
        }
        config.music_player_style = "vu-fullscreen".into();
        player
            .commands
            .send(PlayerCommand::Configure(Box::new(config)))
            .await?;
        player
            .commands
            .send(PlayerCommand::SetPaused(false))
            .await?;
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.audio_levels.is_some() && state.position > 0.4 && !state.paused {
                break;
            }
            player.status.changed().await?;
        }
        anyhow::ensure!(
            std::fs::read(&launches)?.len() == 1,
            "Switching views restarted playback"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await;
    player.close().await;
    result.expect("Meter test timed out").unwrap();
}
