#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Search,
    Command,
    NextTab,
    PreviousTab,
    Up,
    Down,
    Open,
    Preview,
    Download,
    Pause,
    Remove,
    Retry,
    Back,
    Refresh,
    NextPage,
    Enqueue,
    Favorite,
    SaveMedia,
    Theme,
    Settings,
    PlaybackQueue,
    NextTrack,
    PreviousTrack,
    SeekForward,
    SeekBack,
    VolumeUp,
    VolumeDown,
    Mute,
    Shuffle,
    Repeat,
    Stop,
    Cliamp,
    MoveUp,
    MoveDown,
    Fallback,
    Resume,
    Feeds,
    ToggleHints,
    MusicPlayerStyle,
    PlayerMenu,
    Categories,
    Tracks,
    SpeedUp,
    SpeedDown,
    Fullscreen,
    CycleAudio,
    CycleSubtitles,
    ToggleSubtitles,
    NextChapter,
    PreviousChapter,
    SponsorToggle,
    SponsorUndo,
}
impl Action {
    pub fn name(self) -> &'static str {
        REGISTRY
            .iter()
            .find(|(_, a, _, _)| *a == self)
            .map(|(_, _, n, _)| *n)
            .unwrap_or("")
    }
    pub fn parse(name: &str) -> Option<Self> {
        REGISTRY
            .iter()
            .find(|(_, _, n, _)| *n == name)
            .map(|(_, a, _, _)| *a)
    }
}
pub const REGISTRY: &[(&str, Action, &str, &str)] = &[
    (
        "f4",
        Action::MusicPlayerStyle,
        "music_player_style",
        "Cycle music player: classic / vinyl / VU / orbit",
    ),
    (
        "S",
        Action::SaveMedia,
        "save_media",
        "Save / remove video or song locally",
    ),
    (
        "f1",
        Action::ToggleHints,
        "toggle_hints",
        "Show / hide shortcut hints",
    ),
    ("f2", Action::Command, "command", "Actions panel"),
    ("o", Action::PlayerMenu, "player_menu", "Player controls"),
    (
        "f3",
        Action::Categories,
        "categories",
        "YouTube / Music categories",
    ),
    ("l", Action::Tracks, "tracks", "Station track library"),
    (">", Action::SpeedUp, "speed_up", "Speed +0.25"),
    ("<", Action::SpeedDown, "speed_down", "Speed -0.25"),
    (
        "z",
        Action::Fullscreen,
        "fullscreen",
        "Toggle video fullscreen",
    ),
    ("a", Action::CycleAudio, "cycle_audio", "Next audio track"),
    (
        "c",
        Action::CycleSubtitles,
        "cycle_subtitles",
        "Next subtitle track",
    ),
    (
        "V",
        Action::ToggleSubtitles,
        "toggle_subtitles",
        "Show / hide subtitles",
    ),
    (
        "ctrl+right",
        Action::NextChapter,
        "next_chapter",
        "Next chapter",
    ),
    (
        "ctrl+left",
        Action::PreviousChapter,
        "previous_chapter",
        "Previous chapter",
    ),
    (
        "B",
        Action::SponsorToggle,
        "sponsor_toggle",
        "Toggle SponsorBlock skipping",
    ),
    (
        "U",
        Action::SponsorUndo,
        "sponsor_undo",
        "Replay skipped segment",
    ),
    ("b", Action::Feeds, "feeds", "Browse feeds & library"),
    (
        "u",
        Action::Resume,
        "resume",
        "Resume saved playback position",
    ),
    (
        "Y",
        Action::Fallback,
        "fallback",
        "Retry/play explicitly with yt-dlp",
    ),
    ("q", Action::Quit, "quit", "Quit"),
    ("?", Action::Help, "help", "Keyboard help"),
    ("/", Action::Search, "search", "Search / add URL"),
    ("tab", Action::NextTab, "next_tab", "Next tab"),
    (
        "shift+tab",
        Action::PreviousTab,
        "previous_tab",
        "Previous tab",
    ),
    ("up", Action::Up, "up", "Previous item"),
    ("down", Action::Down, "down", "Next item"),
    ("enter", Action::Open, "open", "Play / open collection"),
    ("v", Action::Preview, "preview", "Muted video preview"),
    ("d", Action::Download, "download", "Download options"),
    (
        "space",
        Action::Pause,
        "pause",
        "Pause playback / selected download",
    ),
    (
        "delete",
        Action::Remove,
        "remove",
        "Remove selected download",
    ),
    ("r", Action::Retry, "retry", "Retry selected download"),
    ("esc", Action::Back, "back", "Back / close preview"),
    ("ctrl+r", Action::Refresh, "refresh", "Refresh page"),
    ("n", Action::NextPage, "next_page", "Load more"),
    ("e", Action::Enqueue, "enqueue", "Queue selected media"),
    ("f", Action::Favorite, "favorite", "Favorite radio station"),
    ("t", Action::Theme, "theme", "Choose theme"),
    (",", Action::Settings, "settings", "Settings"),
    (
        "p",
        Action::PlaybackQueue,
        "playback_queue",
        "Playback queue",
    ),
    ("]", Action::NextTrack, "next_track", "Next track"),
    (
        "[",
        Action::PreviousTrack,
        "previous_track",
        "Previous track",
    ),
    (
        "right",
        Action::SeekForward,
        "seek_forward",
        "Seek +5 seconds",
    ),
    ("left", Action::SeekBack, "seek_back", "Seek -5 seconds"),
    ("+", Action::VolumeUp, "volume_up", "Volume up"),
    ("-", Action::VolumeDown, "volume_down", "Volume down"),
    ("m", Action::Mute, "mute", "Mute"),
    ("s", Action::Shuffle, "shuffle", "Shuffle queue"),
    ("R", Action::Repeat, "repeat", "Repeat queue"),
    ("x", Action::Stop, "stop", "Stop playback"),
    ("C", Action::Cliamp, "cliamp", "Open installed cliamp"),
    ("ctrl+up", Action::MoveUp, "move_up", "Move download up"),
    (
        "ctrl+down",
        Action::MoveDown,
        "move_down",
        "Move download down",
    ),
];

/// Keys use the same names emitted by the terminal dispatcher.
pub fn valid_key(value: &str) -> bool {
    let value = value
        .strip_prefix("ctrl+")
        .or_else(|| value.strip_prefix("alt+"))
        .unwrap_or(value);
    value.chars().count() == 1
        || [
            "space",
            "enter",
            "tab",
            "shift+tab",
            "esc",
            "up",
            "down",
            "left",
            "right",
            "delete",
        ]
        .contains(&value)
        || value
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n))
}
