mod support {
    pub mod terminal;
}
use std::path::Path;
use support::terminal::Capture;

#[test]
fn real_terminal_opens_every_popup_edits_settings_resizes_and_restores_flags() {
    let temp = tempfile::tempdir().unwrap();
    let mut term = Capture::new(Path::new(env!("CARGO_BIN_EXE_ytui")), temp.path(), false).unwrap();
    for tab in ["1", "2", "3", "4"] {
        term.send(tab).unwrap();
        assert!(term.text().contains("Workspaces"));
    }
    term.send("2").unwrap();
    for (command, expected) in [
        ("help", "Keyboard & navigation"),
        ("theme", "Themes"),
        ("settings", "General settings"),
        ("keys", "Keyboard shortcuts"),
        ("auth", "Connect your YouTube account"),
        ("resume", "Continue watching"),
        ("watches", "Collection"),
        ("stats", "Downloads · all"),
        ("downloads failed", "Downloads · failed"),
        ("groups", "Download groups"),
        ("log", "Diagnostics"),
        ("update", "Update ytui"),
        ("feeds", "Browse feeds"),
        (
            "add https://www.youtube.com/watch?v=fixture1234",
            "Video quality",
        ),
    ] {
        term.command(command, expected).unwrap();
        assert!(
            term.text().contains(expected),
            "{command} did not open expected popup: {}",
            term.text()
        );
        term.send("\x1b").unwrap();
    }
    term.send("p").unwrap();
    assert!(term.text().contains("Playback queue"));
    term.send("\x1b").unwrap();
    term.command("settings", "General settings").unwrap();
    for section in [
        "General",
        "Appearance",
        "Downloads",
        "Subtitles & media",
        "Network & account",
        "Schedule",
        "Tools",
        "Radio",
        "Playback",
    ] {
        assert!(term.text().contains(section));
        term.send("\x1b[6~").unwrap();
    }
    term.send("\x1b").unwrap();
    term.send("/qvds日本語").unwrap();
    term.wait_for("qvds日本語").unwrap();
    assert!(term.text().contains("qvds日本語"));
    term.send("\x1b").unwrap();
    for (cols, rows) in [(80, 24), (35, 12), (212, 58)] {
        term.resize(cols, rows).unwrap();
        term.command("settings", "General settings").unwrap();
        assert!(term.text().contains("General settings"));
        term.send("\x1b").unwrap();
    }
    term.close().unwrap();
}

#[test]
fn real_terminal_uses_german_search_settings_and_offline_lyrics() {
    let temp = tempfile::tempdir().unwrap();
    let mut term = Capture::with_language(
        Path::new(env!("CARGO_BIN_EXE_ytui")),
        temp.path(),
        false,
        "de",
    )
    .unwrap();
    term.command("settings", "Allgemeine Einstellungen")
        .unwrap();
    assert!(term.text().contains("Sprache"));
    term.send("\x1b").unwrap();
    let path = temp.path().join("Offline song.lrc");
    std::fs::write(&path, "[00:01]Fictional offline lyric").unwrap();
    term.command(
        &format!("lyrics-local {}", path.display()),
        "Fictional offline lyric",
    )
    .unwrap();
    assert!(term.text().contains("Liedtext"));
    term.send("\x1b").unwrap();
    term.send(":").unwrap();
    term.send("Liedtext suchen").unwrap();
    term.send("\r").unwrap();
    term.wait_for("an LRCLIB gesendet").unwrap();
    assert!(term.text().contains("Keine Cookies"));
    term.send("\x1b").unwrap();
    term.resize(80, 24).unwrap();
    term.command("settings", "Allgemeine Einstellungen")
        .unwrap();
    term.close().unwrap();
}

#[test]
fn real_terminal_opens_sleep_timer_and_local_saved_shelf_offline() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let store = ytui_services::store::Store::open(&temp.path().join("state")).unwrap();
        let mut snapshot = ytui_services::store::Snapshot::default();
        snapshot
            .saved_media
            .toggle(ytui_core::Media {
                url: "https://youtu.be/abcdefghijk".into(),
                title: "PTY saved fixture".into(),
                ..Default::default()
            })
            .unwrap();
        store.save(snapshot).await.unwrap();
        store.close().await.unwrap();
    });
    let mut term = Capture::new(Path::new(env!("CARGO_BIN_EXE_ytui")), temp.path(), true).unwrap();
    term.command("saved", "PTY saved fixture").unwrap();
    term.send("S").unwrap();
    term.wait_for("Removed from saved shelf").unwrap();
    term.command("sleep 30m", "Sleep timer set").unwrap();
    term.command("sleep", "Sleep timer").unwrap();
    term.send("\x1b").unwrap();
    term.command("sleep off", "Sleep timer cancelled").unwrap();
    term.close().unwrap();
    runtime.block_on(async {
        let store = ytui_services::store::Store::open(&temp.path().join("state")).unwrap();
        assert!(store.load().await.unwrap().saved_media.entries.is_empty());
        store.close().await.unwrap();
    });
}
