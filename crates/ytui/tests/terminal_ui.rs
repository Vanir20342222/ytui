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
        ("help", "Keyboard & commands"),
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
    term.send(":settings\r").unwrap();
    for section in [
        "General",
        "Appearance",
        "Downloads",
        "Subtitles & media",
        "Network & account",
        "Schedule",
        "Tools",
        "Radio",
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
        term.send(":settings\r").unwrap();
        assert!(term.text().contains("General settings"));
        term.send("\x1b").unwrap();
    }
    term.close().unwrap();
}
