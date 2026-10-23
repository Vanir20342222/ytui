//! Discoverable UI routes. Commands are internal routing identifiers; users choose
//! labeled actions and fill individual fields instead of typing shell syntax.
pub struct Entry {
    pub group: &'static str,
    pub title: &'static str,
    pub command: &'static str,
    pub fields: &'static [(&'static str, &'static str)],
}
macro_rules! entry {
    ($g:literal,$t:literal,$c:literal) => {
        Entry {
            group: $g,
            title: $t,
            command: $c,
            fields: &[],
        }
    };
    ($g:literal,$t:literal,$c:literal,$f:expr) => {
        Entry {
            group: $g,
            title: $t,
            command: $c,
            fields: $f,
        }
    };
}
pub const ENTRIES: &[Entry] = &[
    entry!("Settings", "General settings", "settings"),
    entry!("Settings", "Setup & tool capabilities", "capabilities"),
    entry!("Settings", "Inspect thumbnail cache", "thumbnail-cache"),
    entry!("Settings", "Clear thumbnail cache", "thumbnail-cache-clear"),
    entry!(
        "Settings",
        "Export safe support report",
        "support-export",
        &[("New JSON file", "ytui-support.json")]
    ),
    entry!("Settings", "Playback and SponsorBlock", "playback-settings"),
    entry!(
        "Player",
        "SponsorBlock & ad handling status",
        "sponsor-status"
    ),
    entry!(
        "Player",
        "Available formats, audio tracks & captions",
        "player-formats"
    ),
    entry!("Settings", "Keyboard bindings", "keys"),
    entry!("Settings", "Choose theme", "theme"),
    entry!(
        "Settings",
        "Choose theme by name",
        "theme",
        &[("Theme name", "ytui-dark")]
    ),
    entry!("Account", "Connect browser or cookie file", "auth"),
    entry!("Account", "Disconnect saved browser session", "disconnect"),
    entry!(
        "Account",
        "Cancel pending account/settings operation",
        "cancel-operation"
    ),
    entry!("Browse", "Choose feed or source", "feeds"),
    entry!("Browse", "Home recommendations", "home"),
    entry!("Browse", "Subscriptions", "subscriptions"),
    entry!("Browse", "Watch Later", "later"),
    entry!("Browse", "Liked videos or songs", "liked"),
    entry!("Browse", "Library", "library"),
    entry!("Browse", "YouTube watch history", "history"),
    entry!("Browse", "Feed categories", "categories"),
    entry!("Browse", "Selected video details", "details"),
    entry!("Browse", "Related videos", "related"),
    entry!("Browse", "Open selected item in browser", "open"),
    entry!("Browse", "Open channel", "channel", &[("Channel ID", "")]),
    entry!(
        "Browse",
        "Open playlist",
        "playlist",
        &[("Playlist ID", "")]
    ),
    entry!(
        "Browse",
        "Open collection",
        "collection",
        &[("Collection ID", "")]
    ),
    entry!("Music", "Search songs", "songs", &[("Search", "")]),
    entry!("Music", "Search albums", "albums", &[("Search", "")]),
    entry!("Music", "Search artists", "artists", &[("Search", "")]),
    entry!("Music", "Search videos", "videos", &[("Search", "")]),
    entry!("Music", "Search playlists", "playlists", &[("Search", "")]),
    entry!("Library", "Downloaded videos", "offline video"),
    entry!("Library", "Downloaded music", "offline audio"),
    entry!("Library", "Continue watching", "resume"),
    entry!("Library", "Clear saved playback positions", "resume clear"),
    entry!("Library", "Collection watches", "watches"),
    entry!(
        "Library",
        "Watch this collection for new uploads",
        "watch add",
        &[("Check interval", "30m")]
    ),
    entry!(
        "Downloads",
        "Add URL",
        "add",
        &[("Video or playlist URL", "")]
    ),
    entry!(
        "Downloads",
        "Download options for selection",
        "download-options"
    ),
    entry!("Downloads", "All downloads and history", "downloads all"),
    entry!("Downloads", "Completed downloads", "downloads completed"),
    entry!("Downloads", "Failed downloads", "downloads failed"),
    entry!("Downloads", "Download statistics", "stats"),
    entry!(
        "Downloads",
        "Download to a recent folder",
        "download-recent"
    ),
    entry!(
        "Downloads",
        "Download full collection as individual jobs",
        "download-collection",
        &[("Available entry indices (all or 1,3:7)", "all")]
    ),
    entry!(
        "Downloads",
        "Cancel collection loading",
        "cancel-collection"
    ),
    entry!(
        "Downloads",
        "Files in selected download / playlist",
        "download-files"
    ),
    entry!(
        "Downloads",
        "Search download history",
        "downloads",
        &[("Search", "")]
    ),
    entry!("Downloads", "Download groups", "groups"),
    entry!(
        "Downloads",
        "Download selected page entries",
        "download-page",
        &[("Indices or ranges", "all")]
    ),
    entry!("Downloads", "Pause all downloads", "pauseall"),
    entry!("Downloads", "Resume all downloads", "resumeall"),
    entry!("Downloads", "Cancel all downloads", "stopall"),
    entry!("Downloads", "Clear completed jobs", "clear"),
    entry!("Downloads", "Import queue", "import", &[("Input file", "")]),
    entry!(
        "Downloads",
        "Export queue and history",
        "export",
        &[("Output file", "")]
    ),
    entry!(
        "Downloads",
        "Convert local media",
        "convert",
        &[("Input file", "")]
    ),
    entry!(
        "Downloads",
        "Schedule pending jobs",
        "schedule",
        &[("Delay or time", "30m")]
    ),
    entry!("Radio", "Cliamp live stations", "cliamp"),
    entry!(
        "Radio",
        "Search radio stations",
        "radio",
        &[("Station name", "")]
    ),
    entry!(
        "Radio",
        "Browse by country",
        "country",
        &[("Country code", "DE")]
    ),
    entry!("Radio", "Browse by genre or tag", "tag", &[("Tag", "jazz")]),
    entry!("Radio", "Favorite stations", "favorites"),
    entry!("Radio", "Custom stations", "custom"),
    entry!(
        "Radio",
        "Add custom live station",
        "add-station",
        &[("Station name", ""), ("Stream URL", "")]
    ),
    entry!("Radio", "Track library of selected station", "tracks"),
    entry!(
        "Radio",
        "Pin country",
        "pin-country",
        &[("Country code", "DE")]
    ),
    entry!(
        "Radio",
        "Unpin country",
        "unpin-country",
        &[("Country code", "DE")]
    ),
    entry!("Radio", "Launch installed cliamp", "launch-cliamp"),
    entry!(
        "Player",
        "Video quality",
        "player-quality",
        &[("Quality: best or height", "1080")]
    ),
    entry!("Player", "Play or pause", "player-pause"),
    entry!("Player", "Playback queue", "player-queue"),
    entry!("Player", "Next track", "player-next"),
    entry!("Player", "Previous track", "player-previous"),
    entry!(
        "Player",
        "Seek to position in seconds",
        "player-seek",
        &[("Seconds", "0")]
    ),
    entry!("Player", "Volume up", "player-volume-up"),
    entry!("Player", "Volume down", "player-volume-down"),
    entry!("Player", "Mute", "player-mute"),
    entry!("Player", "Speed up", "player-speed-up"),
    entry!("Player", "Speed down", "player-speed-down"),
    entry!("Player", "Fullscreen", "player-fullscreen"),
    entry!("Player", "Next audio track", "player-audio"),
    entry!("Player", "Next subtitle track", "player-subtitle"),
    entry!("Player", "Toggle subtitles", "player-subtitle-toggle"),
    entry!("Player", "Next chapter", "player-chapter-next"),
    entry!("Player", "Previous chapter", "player-chapter-previous"),
    entry!(
        "Player",
        "Toggle SponsorBlock skipping",
        "player-sponsor-toggle"
    ),
    entry!(
        "Player",
        "Replay last skipped segment",
        "player-sponsor-undo"
    ),
    entry!("Player", "Shuffle queue", "player-shuffle"),
    entry!("Player", "Repeat queue", "player-repeat"),
    entry!("Player", "Stop", "player-stop"),
    entry!(
        "Playback repair",
        "Retry using external yt-dlp (Python)",
        "fallback"
    ),
    entry!(
        "Playback repair",
        "Select download extraction backend",
        "backend",
        &[("Backend: auto, native or yt-dlp", "auto")]
    ),
    entry!(
        "Playback repair",
        "Select playback backend",
        "playback-backend",
        &[("Playback: native, auto or yt-dlp", "native")]
    ),
    entry!(
        "Playback repair",
        "Configure native JavaScript runtime",
        "native-runtime",
        &[("Node.js 22+ or Deno 2.9.7 executable", "node")]
    ),
    entry!("Tools", "Diagnostics", "log"),
    entry!("Tools", "Check for updates", "update"),
    entry!("Tools", "List saved settings profiles", "profiles"),
    entry!(
        "Tools",
        "Save settings profile",
        "profile save",
        &[("Profile name", "")]
    ),
    entry!(
        "Tools",
        "Load settings profile",
        "profile load",
        &[("Profile name", "")]
    ),
    entry!(
        "Tools",
        "Delete settings profile",
        "profile delete",
        &[("Profile name", "")]
    ),
    entry!("Tools", "VPN connections", "vpn list"),
    entry!(
        "Tools",
        "Connect VPN profile",
        "vpn connect",
        &[("Connection UUID", "")]
    ),
    entry!(
        "Tools",
        "Disconnect VPN profile",
        "vpn disconnect",
        &[("Connection UUID", "")]
    ),
    entry!("Interface", "Toggle shortcut hints", "hints"),
    entry!("Interface", "Keyboard help", "help"),
];
pub fn matching(query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    ENTRIES
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            let haystack = format!("{} {} {}", e.group, e.title, e.command).to_lowercase();
            query.split_whitespace().all(|part| haystack.contains(part))
        })
        .map(|(i, _)| i)
        .collect()
}
