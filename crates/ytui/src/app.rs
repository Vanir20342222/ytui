use crate::{
    editor::Editor,
    forms::{Form, FormKind},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui_image::{Resize, picker::Picker, protocol::Protocol};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use ytui_core::{
    shortcuts::{Action, REGISTRY},
    *,
};
use ytui_services::{
    auth::{self, Session, SessionStore},
    browse::Browser,
    download,
    images::ImageCache,
    paths::Paths,
    player::{PlayerCommand, PlayerStatus},
    radio::Radio,
    store::Snapshot,
    theme::{self, Palette},
};

pub const TAB_NAMES: [&str; 4] = ["Downloads", "YouTube", "Music", "Cliamp / Radio"];
fn image_protocol(
    value: &str,
    detected: ratatui_image::picker::ProtocolType,
) -> ratatui_image::picker::ProtocolType {
    use ratatui_image::picker::ProtocolType;
    match value {
        "auto" => detected,
        "kitty" => ProtocolType::Kitty,
        "sixel" => ProtocolType::Sixel,
        "iterm2" => ProtocolType::Iterm2,
        _ => ProtocolType::Halfblocks,
    }
}
fn image_retry(error: &anyhow::Error, attempts: u8) -> Option<Instant> {
    let message = format!("{error:#}").to_lowercase();
    let transient = [
        "timeout",
        "timed out",
        "connection",
        "temporary",
        "http 5",
        "http 429",
        "error sending",
        "cannot reach",
    ]
    .iter()
    .any(|part| message.contains(part));
    (transient && attempts <= 5).then(|| {
        Instant::now()
            + Duration::from_secs(
                [2, 5, 15, 30, 60][usize::from(attempts.saturating_sub(1)).min(4)],
            )
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    SettingSection(usize),
    SettingField(usize),
    SettingsSave,
    SettingsCancel,
    Tab(usize),
    Feed(usize),
    Feeds,
    Item(usize),
    Catalog,
    MenuItem(usize),
    Action(Action),
    Category(usize),
    Timeline,
}
pub struct FeedLoad {
    pub request: BrowseRequest,
    pub append: bool,
    pub preserve_selection: bool,
    pub failures: u32,
    pub retry_at: Option<Instant>,
}
#[derive(Default)]
pub struct TabView {
    pub page: Page,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub request: Option<BrowseRequest>,
    pub generation: u64,
    pub cancel: CancellationToken,
    pub history: Vec<(BrowseRequest, Page, usize)>,
    pub load: Option<FeedLoad>,
    pub empty_pages: u8,
    pub seen_tokens: std::collections::HashSet<String>,
}
impl TabView {
    fn bound_memory(&mut self) {
        let mut bytes = self
            .page
            .items
            .iter()
            .map(Media::retained_bytes)
            .sum::<usize>();
        let mut remove = 0;
        while self.page.items.len().saturating_sub(remove) > 2000 || bytes > 8 * 1024 * 1024 {
            let Some(media) = self.page.items.get(remove) else {
                break;
            };
            bytes = bytes.saturating_sub(media.retained_bytes());
            remove += 1;
        }
        if remove > 0 {
            self.page.items.drain(..remove);
            self.selected = self.selected.saturating_sub(remove);
            self.page.notice = Some(
                "Continuous feed · older items released from memory (2,000 items / 8 MiB window)"
                    .into(),
            );
        }
        while self.history.len() > 8
            || self
                .history
                .iter()
                .map(|(_, page, _)| page.retained_bytes())
                .sum::<usize>()
                > 12 * 1024 * 1024
        {
            self.history.remove(0);
        }
        // Cycle detection covers the recent traversal. A bounded set avoids
        // retaining every continuation token during an arbitrarily long feed.
        if self.seen_tokens.len() > 256 {
            self.seen_tokens.clear();
        }
    }
}
pub enum Modal {
    PendingText {
        id: uuid::Uuid,
        title: String,
    },
    Lyrics(Box<crate::lyrics::View>),
    Actions {
        selected: usize,
        query: Editor,
    },
    Categories(usize),
    Feeds(usize),
    Help(usize),
    Themes(usize),
    Form(Box<Form>),
    Queue(usize),
    Resume(usize),
    Watches(usize),
    History(history::View),
    Groups(usize),
    Destinations(usize),
    PlayerSettings {
        section: usize,
        selected: usize,
    },
    Account {
        profiles: Vec<auth::BrowserProfile>,
        selected: usize,
    },
    Outputs {
        job: uuid::Uuid,
        selected: usize,
    },
    Text {
        title: String,
        lines: Vec<String>,
        offset: usize,
    },
}
pub struct Input {
    pub editor: Editor,
}
pub enum UiEvent {
    Lyrics {
        id: uuid::Uuid,
        result: Result<Vec<ytui_services::lyrics::Track>>,
    },
    Page {
        tab: usize,
        generation: u64,
        append: bool,
        result: Result<Page>,
    },
    Prefetched,
    CardImage {
        generation: u64,
        key: String,
        result: Result<Protocol>,
    },
    Image {
        generation: u64,
        result: Result<Protocol>,
    },
    Config {
        operation: crate::operations::Ticket,
        config: Box<Config>,
        account: Option<Session>,
        result: Result<()>,
    },
    Auth {
        operation: crate::operations::Ticket,
        result: Result<Session>,
    },
    Disconnected {
        operation: crate::operations::Ticket,
        result: Result<()>,
    },
    Status(String),
    Clipboard {
        generation: u64,
        result: Result<String>,
    },
    Notification(Result<()>),
    Import(Result<Vec<Job>>),
    Watch {
        id: uuid::Uuid,
        generation: u64,
        result: Result<Vec<Media>>,
    },
    Text {
        id: uuid::Uuid,
        result: Result<Vec<String>>,
    },
    CacheCleared(Result<ytui_services::images::CacheStats>),
    AccountProfiles(Vec<auth::BrowserProfile>),
    Collection {
        generation: u64,
        source: String,
        title: String,
        options: Box<DownloadOptions>,
        selection: String,
        result: Result<Vec<Media>>,
    },
}
pub struct RuntimeHandles {
    pub player: mpsc::Sender<PlayerCommand>,
    pub jobs: mpsc::Sender<JobEvent>,
    pub picker: Picker,
}
pub struct Running {
    pub cancel: CancellationToken,
    pub task: JoinHandle<()>,
}
pub struct App {
    pub operations: crate::operations::Operations,
    pub pending_operation: Option<(crate::operations::Ticket, String)>,
    disconnect_pending: bool,
    collection_cancel: CancellationToken,
    collection_generation: u64,
    pub collection_pending: bool,
    pending_auto_play: Option<Media>,
    pub sleep_timer: crate::sleep::SleepTimer,
    pub player_session_pending: bool,
    pub config: Config,
    pub paths: Paths,
    pub snapshot: Snapshot,
    pub tab: usize,
    pub tabs: [TabView; 4],
    pub download_selected: usize,
    pub modal: Option<Modal>,
    pub input: Option<Input>,
    pub status: String,
    pub palette: Palette,
    pub player_status: PlayerStatus,
    pub vinyl_motion: crate::ui::vinyl::Motion,
    pub visual_levels: Option<ytui_services::visualizer::AudioLevels>,
    pub demo: bool,
    pub offline: bool,
    pub quit: bool,
    pub launch_cliamp: bool,
    pub dirty: bool,
    pub redraw: bool,
    pub tasks: JoinSet<UiEvent>,
    pub running: HashMap<uuid::Uuid, Running>,
    pub job_tx: mpsc::Sender<JobEvent>,
    pub player: mpsc::Sender<PlayerCommand>,
    pub browser: Browser,
    pub radio: Radio,
    pub session: Option<Arc<Session>>,
    pub theme_rx: tokio::sync::watch::Receiver<Palette>,
    theme_cancel: CancellationToken,
    pub picker: Picker,
    pub detected_image_protocol: ratatui_image::picker::ProtocolType,
    pub image: Option<Protocol>,
    pub image_area: Rect,
    pub card_areas: Vec<(usize, Rect)>,
    pub card_images: HashMap<String, Protocol>,
    card_requests: std::collections::HashSet<String>,
    card_failures: HashMap<String, (u8, Option<Instant>)>,
    card_generation: u64,
    card_cancel: CancellationToken,
    card_pending: usize,
    image_key: Option<(String, u16, u16)>,
    image_generation: u64,
    image_cancel: CancellationToken,
    image_due: Instant,
    image_failures: u8,
    pub image_error: Option<String>,
    pub sidebar_visible: bool,
    pub feeds_focused: bool,
    pub feed_selected: usize,
    pub hit_targets: Vec<(Rect, HitTarget)>,
    images: ImageCache,
    policy: Option<schedule::Policy>,
    desktop_cancel: CancellationToken,
    clipboard_cancel: CancellationToken,
    clipboard_monitor: ytui_services::desktop::ClipboardMonitor,
    clipboard_generation: u64,
    clipboard_pending: bool,
    clipboard_due: Instant,
    pub clipboard_url: Option<String>,
    clipboard_warned: bool,
    notification_pending: usize,
    notification_warned: bool,
    watch_pending: HashMap<uuid::Uuid, (u64, CancellationToken)>,
}
impl App {
    fn pending_text(&mut self, title: &str) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        self.modal = Some(Modal::PendingText {
            id,
            title: title.into(),
        });
        id
    }
    pub fn tr<'a>(&self, text: &'a str) -> &'a str {
        crate::i18n::text(&self.config.language, text)
    }
    pub fn card_notice(&self, key: &str) -> Option<&'static str> {
        self.card_failures.get(key).map(|(_, due)| {
            if due.is_some() {
                "Retrying image…"
            } else {
                "No image · F5 retries"
            }
        })
    }
    fn remember_destination(&mut self, options: &DownloadOptions) {
        let path = options.output_directory();
        if path.as_os_str().is_empty() {
            return;
        }
        self.snapshot
            .recent_destinations
            .retain(|previous| previous != path);
        self.snapshot.recent_destinations.insert(0, path.to_owned());
        self.snapshot.recent_destinations.truncate(12);
    }
    pub fn playback_choices(&self, section: usize) -> Vec<(String, PlayerCommand)> {
        match section {
            0 => self
                .player_status
                .catalog
                .formats
                .iter()
                .map(|format| (format.label.clone(), PlayerCommand::Format(format.clone())))
                .collect(),
            1 => self
                .player_status
                .tracks
                .iter()
                .map(|track| {
                    (
                        format!("{} {}", if track.selected { "●" } else { "○" }, track.label),
                        PlayerCommand::SelectTrack {
                            kind: track.kind.clone(),
                            id: track.id,
                        },
                    )
                })
                .collect(),
            _ => self
                .player_status
                .catalog
                .captions
                .iter()
                .map(|caption| {
                    (
                        caption.label.clone(),
                        PlayerCommand::Caption(caption.clone()),
                    )
                })
                .collect(),
        }
    }
    fn continue_disconnect(&mut self) {
        if self.disconnect_pending && self.pending_operation.is_none() {
            self.disconnect_pending = false;
            if let Err(error) = self.command("disconnect") {
                self.status = error.to_string();
            }
        }
    }
    fn finish_operation(&mut self, operation: &crate::operations::Ticket) -> bool {
        if !self.operations.current(operation) {
            return false;
        }
        if self
            .pending_operation
            .as_ref()
            .is_some_and(|(pending, _)| pending.same(operation))
        {
            self.pending_operation = None;
        }
        true
    }
    pub fn feed_choices(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        if self.offline {
            return &[
                (
                    "Downloaded videos",
                    "Completed videos on this device",
                    "offline video",
                ),
                (
                    "Downloaded music",
                    "Completed audio on this device",
                    "offline audio",
                ),
                (
                    "Saved locally",
                    "Your local video and song bookmarks",
                    "saved",
                ),
                (
                    "Continue watching",
                    "Your locally saved playback positions",
                    "resume",
                ),
                (
                    "Collection watches",
                    "Manage watches; polling waits for online mode",
                    "watches",
                ),
            ];
        }
        match self.tab {
            0 => &[
                (
                    "Saved locally",
                    "Your local video and song bookmarks",
                    "saved",
                ),
                (
                    "Completed downloads",
                    "Search and filter your download history",
                    "downloads completed",
                ),
                (
                    "Offline library",
                    "Play media already on this device",
                    "offline",
                ),
                (
                    "Continue watching",
                    "Resume a locally saved position",
                    "resume",
                ),
                (
                    "Collection watches",
                    "Manage automatic collection checks",
                    "watches",
                ),
                ("Statistics", "Completed media and transfer totals", "stats"),
            ],
            2 => &[
                ("Music home", "Recommendations from YouTube Music", "home"),
                (
                    "Saved locally",
                    "Your local video and song bookmarks",
                    "saved",
                ),
                ("Your library", "Saved albums and playlists", "library"),
                ("Saved albums", "Albums in your Music library", "albums"),
                ("Artists", "Artists in your Music library", "artists"),
                ("Liked music", "Your liked songs", "liked"),
                ("Recently played", "Your YouTube Music history", "history"),
                (
                    "Downloaded music",
                    "Completed audio on this device",
                    "offline audio",
                ),
                (
                    "Continue listening",
                    "Resume a locally saved position",
                    "resume",
                ),
            ],
            3 => &[
                ("Cliamp channels", "Streams and track libraries", "cliamp"),
                ("Discover stations", "Explore Radio Browser", "radio"),
                (
                    "Favorite stations",
                    "Stations you saved in ytui",
                    "favorites",
                ),
            ],
            _ => &[
                ("For you", "Your YouTube home feed", "home"),
                (
                    "Saved locally",
                    "Your local video and song bookmarks",
                    "saved",
                ),
                (
                    "Subscriptions",
                    "Latest from channels you follow",
                    "subscriptions",
                ),
                ("Watch later", "Videos saved for another time", "later"),
                ("Liked videos", "Your liked videos", "liked"),
                ("Your library", "Playlists and collections", "library"),
                ("Watch history", "Your YouTube watch history", "history"),
                (
                    "Offline library",
                    "Completed media on this device",
                    "offline",
                ),
                (
                    "Continue watching",
                    "Resume a locally saved position",
                    "resume",
                ),
            ],
        }
    }
    pub fn new(
        config: Config,
        paths: Paths,
        snapshot: Snapshot,
        session: Option<Arc<Session>>,
        handles: RuntimeHandles,
        demo: bool,
        offline: bool,
    ) -> Result<Self> {
        let RuntimeHandles {
            player,
            jobs: job_tx,
            picker,
        } = handles;
        let detected_image_protocol = picker.protocol_type();
        let mut picker = picker;
        picker.set_protocol_type(image_protocol(
            &config.image_protocol,
            detected_image_protocol,
        ));
        let client = ytui_services::http_client(config.proxy.as_deref())?;
        let browser = Browser::new(client.clone(), session.clone(), config.account_index);
        let radio = Radio::new(client.clone(), config.radio_servers.clone())
            .with_cache(paths.cache.join("radio"));
        let images = ImageCache::new(client, paths.cache.join("images"));
        let theme_cancel = CancellationToken::new();
        let theme_rx = theme::watch(
            Arc::new(config.clone()),
            paths.clone(),
            theme_cancel.clone(),
        );
        let mut app = Self {
            operations: Default::default(),
            pending_operation: None,
            disconnect_pending: false,
            collection_cancel: CancellationToken::new(),
            collection_generation: 0,
            collection_pending: false,
            pending_auto_play: None,
            sleep_timer: Default::default(),
            player_session_pending: false,
            config,
            paths,
            snapshot,
            tab: 1,
            tabs: std::array::from_fn(|_| TabView::default()),
            download_selected: 0,
            modal: None,
            input: None,
            status: if offline {
                "Offline · / search local media · u resume · F2 actions".into()
            } else if session.is_some() {
                "Connected · b feeds · F2 actions · , settings".into()
            } else {
                "Anonymous · F2 actions · , settings · ? help".into()
            },
            palette: theme::builtin("ytui-dark")?,
            player_status: PlayerStatus::default(),
            demo,
            offline,
            quit: false,
            launch_cliamp: false,
            dirty: false,
            redraw: true,
            vinyl_motion: Default::default(),
            visual_levels: None,
            tasks: JoinSet::new(),
            running: HashMap::new(),
            job_tx,
            player,
            browser,
            radio,
            session,
            theme_rx,
            theme_cancel,
            picker,
            detected_image_protocol,
            image: None,
            image_area: Rect::default(),
            card_areas: Vec::new(),
            card_images: HashMap::new(),
            card_requests: std::collections::HashSet::new(),
            card_failures: HashMap::new(),
            card_generation: 0,
            card_cancel: CancellationToken::new(),
            card_pending: 0,
            image_key: None,
            image_generation: 0,
            image_cancel: CancellationToken::new(),
            image_due: Instant::now(),
            image_failures: 0,
            image_error: None,
            sidebar_visible: false,
            feeds_focused: false,
            feed_selected: 0,
            hit_targets: Vec::new(),
            images,
            policy: None,
            desktop_cancel: CancellationToken::new(),
            clipboard_cancel: CancellationToken::new(),
            clipboard_monitor: Default::default(),
            clipboard_generation: 0,
            clipboard_pending: false,
            clipboard_due: Instant::now(),
            clipboard_url: None,
            clipboard_warned: false,
            notification_pending: 0,
            notification_warned: false,
            watch_pending: HashMap::new(),
        };
        if demo {
            app.load_demo();
        } else if offline {
            app.navigate(
                1,
                BrowseRequest::OfflineLibrary {
                    query: String::new(),
                    audio: None,
                },
                false,
            );
        } else {
            app.navigate(1, BrowseRequest::Home, false);
        }
        let cache = app.images.clone();
        app.tasks.spawn(async move {
            let _ = cache.prune().await;
            UiEvent::Status(String::new())
        });
        Ok(app)
    }
    fn load_demo(&mut self) {
        for tab in 1..4 {
            self.tabs[tab].page = Page {
                title: if tab == 3 {
                    "cliamp radio"
                } else if tab == 2 {
                    "Your music"
                } else {
                    "For you"
                }
                .into(),
                items: (0..18)
                    .map(|i| Media {
                        id: format!("demo-{i}"),
                        thumbnail: format!("demo://art/{i}"),
                        title: [
                            "A quieter corner of the internet",
                            "Late night, soft keys",
                            "Building something that lasts",
                            "A walk through the northern lights",
                            "The art of paying attention",
                            "Small discoveries, every day",
                        ][i % 6]
                            .into(),
                        creator: if tab == 2 {
                            "Studio Sessions"
                        } else if tab == 3 {
                            "Independent radio"
                        } else {
                            "Everyday Curiosity"
                        }
                        .into(),
                        duration: Some((i as u64 + 1) * 97),
                        description: "Fictional demo content. Press / to explore the interface, t to choose a theme, or ? for shortcuts. No account or network is used for this catalog.".into(),
                        kind: if tab == 2 {
                            MediaKind::Song
                        } else if tab == 3 {
                            MediaKind::Station
                        } else {
                            MediaKind::Video
                        },
                        live: tab == 3,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
        }
        let mut a = Job::new(
            self.tabs[1].page.items[0].clone(),
            self.config.download.clone(),
        );
        a.state = JobState::Paused;
        let mut b = Job::new(
            self.tabs[1].page.items[1].clone(),
            self.config.download.clone(),
        );
        b.state = JobState::Done;
        self.snapshot.downloads.jobs = vec![a, b];
        self.player_status = PlayerStatus {
            audio_levels: Some(ytui_services::visualizer::AudioLevels {
                left: 0.72,
                right: 0.55,
                peak: 0.86,
            }),
            backend: "demo · fictional formats".into(),
            catalog: ytui_services::formats::Catalog::parse(&serde_json::json!({
                "formats":[{"format_id":"137","height":1080,"vcodec":"avc1","acodec":"none","ext":"mp4"},{"format_id":"247","height":720,"vcodec":"vp9","acodec":"none","ext":"webm"},{"format_id":"140-de","vcodec":"none","acodec":"aac","language":"de","ext":"m4a"}],
                "subtitles":{"de":[{"ext":"vtt","url":"https://example.invalid/fictional-caption"}]}
            })),
            tracks: ytui_services::formats::tracks(
                &serde_json::json!([{"id":1,"type":"audio","lang":"en","codec":"aac","selected":true},{"id":2,"type":"audio","lang":"de","codec":"aac"},{"id":1,"type":"sub","lang":"de","codec":"vtt"}]),
            ),
            media: Some(Media {
                title: "Studio Sessions · A quiet evening".into(),
                creator: "Studio Sessions".into(),
                thumbnail: "demo://art/1".into(),
                kind: MediaKind::Song,
                ..Default::default()
            }),
            title: "Studio Sessions · A quiet evening".into(),
            position: 82.0,
            duration: Some(240.0),
            volume: 42.0,
            speed: 1.0,
            active: true,
            sponsor_enabled: true,
            segments: vec![ytui_services::sponsorblock::Segment {
                start: 90.0,
                end: 115.0,
                category: "sponsor".into(),
            }],
            notice: Some(
                "Fictional player state · B toggle skipping · U replay · o controls".into(),
            ),
            ..Default::default()
        };
        self.status = "DEMO · Fictional catalog · No account data".into();
    }
    pub fn selected_for_resume(&self) -> Option<Media> {
        let mut media = self.selected()?;
        if self.tab == 0
            && let Some(job) = self.snapshot.downloads.jobs.get(self.download_selected)
            && job.state == JobState::Done
            && let Some(path) = job.outputs.first()
        {
            media.url = std::fs::canonicalize(path).ok()?.to_string_lossy().into();
            if job.options.audio {
                media.kind = MediaKind::Song;
            }
        }
        Some(media)
    }
    pub fn selected(&self) -> Option<Media> {
        if self.tab == 0 {
            self.snapshot
                .downloads
                .jobs
                .get(self.download_selected)
                .map(|j| j.media.clone())
        } else {
            self.tabs[self.tab]
                .page
                .items
                .get(self.tabs[self.tab].selected)
                .cloned()
        }
    }
    pub fn navigate(&mut self, tab: usize, request: BrowseRequest, append: bool) {
        if tab == 0 {
            return;
        }
        if self.offline
            && !matches!(
                request,
                BrowseRequest::OfflineLibrary { .. } | BrowseRequest::SavedMedia { .. }
            )
        {
            self.status =
                "Offline mode · use :offline or restart without --offline for online feeds".into();
            return;
        }
        if self.demo && !matches!(request, BrowseRequest::SavedMedia { .. }) {
            self.status = "Demo catalog · Network actions are disabled".into();
            return;
        }
        let view = &mut self.tabs[tab];
        let preserve_selection = !append && view.request.as_ref() == Some(&request);
        if !append && !preserve_selection {
            if let Some(old) = view.request.take()
                && !view.page.items.is_empty()
            {
                view.history.push((old, view.page.clone(), view.selected));
                view.bound_memory();
            }
            view.request = Some(request.clone());
        }
        if !append {
            view.empty_pages = 0;
            view.seen_tokens.clear();
        }
        // Manual next-page repeats must not cancel an in-flight request or bypass
        // a rate-limit cooldown. Navigation to another feed always replaces it.
        if append && view.load.is_some() {
            return;
        }
        view.load = Some(FeedLoad {
            request,
            append,
            preserve_selection,
            failures: 0,
            retry_at: None,
        });
        self.start_browse(tab);
    }
    fn start_browse(&mut self, tab: usize) {
        let view = &mut self.tabs[tab];
        let Some(load) = &mut view.load else {
            return;
        };
        let request = load.request.clone();
        let append = load.append;
        load.retry_at = None;
        view.cancel.cancel();
        view.cancel = CancellationToken::new();
        let cancel = view.cancel.clone();
        view.generation += 1;
        let generation = view.generation;
        view.loading = true;
        view.error = None;
        let language = self.config.language.clone();
        let saved = matches!(request, BrowseRequest::SavedMedia { .. })
            .then(|| self.snapshot.saved_media.clone());
        let browser = self.browser.clone();
        let radio = self.radio.clone();
        let favorites = self.snapshot.favorites.clone();
        let custom = self.config.custom_stations.clone();
        let downloads = matches!(request, BrowseRequest::OfflineLibrary { .. })
            .then(|| self.snapshot.downloads.clone());
        let offset = if append {
            view.page
                .continuation
                .as_deref()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        } else {
            0
        };
        self.tasks.spawn(async move {
            let fetch=async move {
                if let BrowseRequest::SavedMedia { query } = &request {
                    let mut page = saved.context("Missing saved shelf")?.catalog(query);
                    page.title = crate::i18n::text(&language, &page.title).into();
                    page.notice = page.notice.map(|text| crate::i18n::text(&language, &text).into());
                    Ok(page)
                } else if let BrowseRequest::OfflineLibrary { query, audio } = &request {
                    let queue = downloads.context("Missing offline queue snapshot")?;
                    let query = query.clone(); let audio = *audio;
                    tokio::task::spawn_blocking(move || ytui_services::library::catalog(&queue, &query, audio)).await.map_err(anyhow::Error::from)
                } else if tab==3 {
                    if request==BrowseRequest::Favorites {Ok(Page{title:"Favorite stations".into(),items:favorites,..Default::default()})}
                    else if request==BrowseRequest::Collection("custom".into()){Ok(Page{title:"Custom stations".into(),items:custom,..Default::default()})}
                    else{radio.fetch(&request,offset).await}
                }else{browser.fetch(tab==2,&request).await}
            };
            let result=tokio::select!{_=cancel.cancelled()=>Err(anyhow::anyhow!("Request cancelled")),r=fetch=>r};
            UiEvent::Page{tab,generation,append,result}
        });
        self.redraw = true;
    }
    pub fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::Lyrics { id, result } => {
                if let Some(Modal::Lyrics(view)) = &mut self.modal
                    && view.id == id
                {
                    view.loading = false;
                    match result {
                        Ok(tracks) => {
                            view.opened = view.source == "Local file";
                            view.tracks = tracks;
                        }
                        Err(error) => {
                            view.error =
                                Some(ytui_services::process::diagnostic(&error.to_string()))
                        }
                    }
                }
            }
            UiEvent::AccountProfiles(profiles) => {
                if matches!(self.modal, Some(Modal::Account { .. })) {
                    self.modal = Some(Modal::Account {
                        profiles,
                        selected: 0,
                    });
                }
            }
            UiEvent::Collection {
                generation,
                source,
                title,
                options,
                selection,
                result,
            } => {
                if generation != self.collection_generation {
                    return;
                }
                self.collection_pending = false;
                match result.and_then(|items| {
                    self.queue_collection(source, title, *options, &selection, items)
                }) {
                    Ok(()) => {}
                    Err(error) => {
                        self.status = format!(
                            "Collection was not queued: {}",
                            ytui_services::process::diagnostic(&error.to_string())
                        )
                    }
                }
            }
            UiEvent::CacheCleared(result) => match result {
                Ok(stats) => {
                    self.card_generation = self.card_generation.wrapping_add(1);
                    self.card_images.clear();
                    self.card_requests.clear();
                    self.card_failures.clear();
                    self.card_pending = 0;
                    self.selection_changed();
                    self.status = format!(
                        "Cleared {} cached thumbnails ({:.1} MiB) · media files unchanged",
                        stats.files,
                        stats.disk_bytes as f64 / 1048576.0
                    );
                }
                Err(error) => self.status = format!("Could not clear thumbnails: {error}"),
            },
            UiEvent::Prefetched => {}
            UiEvent::CardImage {
                generation,
                key,
                result,
            } => {
                if generation != self.card_generation {
                    return;
                }
                self.card_pending = self.card_pending.saturating_sub(1);
                self.card_requests.remove(&key);
                if let Ok(protocol) = result {
                    self.card_failures.remove(&key);
                    let visible = self
                        .card_areas
                        .iter()
                        .filter_map(|(index, rect)| {
                            self.tabs[self.tab]
                                .page
                                .items
                                .get(*index)
                                .map(|media| self.card_key(media, *rect))
                        })
                        .collect::<std::collections::HashSet<_>>();
                    if self.card_images.len() >= 64
                        && let Some(old) = self
                            .card_images
                            .keys()
                            .find(|key| !visible.contains(*key))
                            .cloned()
                    {
                        self.card_images.remove(&old);
                        self.card_requests.remove(&old);
                    }
                    self.card_images.insert(key, protocol);
                } else if let Err(error) = result {
                    let attempts = self
                        .card_failures
                        .get(&key)
                        .map_or(1, |(n, _)| n.saturating_add(1));
                    self.card_failures
                        .insert(key, (attempts, image_retry(&error, attempts)));
                }
            }
            UiEvent::Page {
                tab,
                generation,
                append,
                result,
            } => {
                let view = &mut self.tabs[tab];
                if generation != view.generation {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(page) => {
                        let preserve_selection =
                            view.load.take().is_some_and(|load| load.preserve_selection);
                        view.error = None;
                        if append {
                            let mut ids = view
                                .page
                                .items
                                .iter()
                                .map(|m| m.id.clone())
                                .collect::<std::collections::HashSet<_>>();
                            let before = view.page.items.len();
                            for media in page.items {
                                if ids.insert(media.id.clone()) {
                                    view.page.items.push(media);
                                }
                            }
                            // Recommendation pages may overlap or be empty yet carry a
                            // useful next token. Follow up to three such pages, while
                            // rejecting repeated tokens and cycles (A → B → A).
                            if view.page.items.len() == before {
                                view.empty_pages = view.empty_pages.saturating_add(1);
                            } else {
                                view.empty_pages = 0;
                            }
                            if let Some(token) = view.page.continuation.take() {
                                view.seen_tokens.insert(token);
                            }
                            view.page.continuation = page.continuation.filter(|token| {
                                view.empty_pages < 3 && !view.seen_tokens.contains(token)
                            });
                            if !page.categories.is_empty() {
                                view.page.categories = page.categories;
                            }
                        } else {
                            let selected_id = preserve_selection
                                .then(|| view.page.items.get(view.selected).map(|m| m.id.clone()))
                                .flatten();
                            view.selected = selected_id
                                .and_then(|id| page.items.iter().position(|m| m.id == id))
                                .unwrap_or(if preserve_selection {
                                    view.selected.min(page.items.len().saturating_sub(1))
                                } else {
                                    0
                                });
                            view.page = page;
                        }
                        view.bound_memory();
                        if tab == self.tab && !append {
                            self.selection_changed();
                        }
                        if tab == self.tab {
                            self.prefetch_images();
                        }
                    }
                    Err(e) => {
                        view.error = Some(ytui_services::process::diagnostic(&e.to_string()));
                        if matches!(tab, 1 | 2)
                            && let Some(retry) =
                                e.downcast_ref::<ytui_services::browse::FeedRetry>()
                            && let Some(load) = &mut view.load
                        {
                            let backoff = [2, 5, 15, 30, 60][(load.failures as usize).min(4)];
                            load.failures = load.failures.saturating_add(1);
                            // Obey Retry-After even when it exceeds our normal backoff.
                            let delay = Duration::from_secs(backoff).max(retry.after);
                            load.retry_at =
                                Some(Instant::now().checked_add(delay).unwrap_or_else(|| {
                                    Instant::now() + Duration::from_secs(86400)
                                }));
                        } else {
                            view.load = None;
                        }
                    }
                }
            }
            UiEvent::Image { generation, result } => {
                if generation != self.image_generation {
                    return;
                }
                match result {
                    Ok(p) => {
                        self.image = Some(p);
                        self.image_error = None;
                    }
                    Err(error) => {
                        self.image = None;
                        self.image_error = Some("Thumbnail unavailable".into());
                        self.image_failures = self.image_failures.saturating_add(1);
                        if let Some(due) = image_retry(&error, self.image_failures) {
                            self.image_key = None;
                            self.image_due = due;
                            self.image_error = Some("Thumbnail retry scheduled".into());
                        }
                    }
                }
            }
            UiEvent::Config {
                operation,
                config,
                account,
                result,
            } => {
                if !self.finish_operation(&operation) {
                    return;
                }
                match result {
                    Err(e) => self.status = format!("Could not save settings: {e}"),
                    Ok(()) => {
                        let account_changed = self.config.account_index != config.account_index;
                        let network_changed = self.config.proxy != config.proxy;
                        let radio_changed = self.config.radio_servers != config.radio_servers;
                        let clipboard_changed =
                            self.config.clipboard_watcher != config.clipboard_watcher;
                        let images_changed = self.config.image_protocol != config.image_protocol;
                        self.config = *config;
                        if clipboard_changed {
                            self.clipboard_cancel.cancel();
                            self.clipboard_cancel = CancellationToken::new();
                            self.clipboard_generation += 1;
                            self.clipboard_monitor = Default::default();
                            self.clipboard_url = None;
                            self.clipboard_warned = false;
                        }
                        if images_changed {
                            self.picker.set_protocol_type(image_protocol(
                                &self.config.image_protocol,
                                self.detected_image_protocol,
                            ));
                            self.card_generation = self.card_generation.wrapping_add(1);
                            self.card_images.clear();
                            self.card_requests.clear();
                            self.card_failures.clear();
                            self.card_pending = 0;
                        }
                        if let Some(account) = account {
                            self.session = Some(Arc::new(account));
                        }
                        let _ = self
                            .player
                            .try_send(PlayerCommand::Configure(Box::new(self.config.clone())));
                        self.theme_cancel.cancel();
                        self.theme_cancel = CancellationToken::new();
                        self.theme_rx = theme::watch(
                            Arc::new(self.config.clone()),
                            self.paths.clone(),
                            self.theme_cancel.clone(),
                        );
                        match ytui_services::http_client(self.config.proxy.as_deref()) {
                            Ok(client) => {
                                self.browser = Browser::new(
                                    client.clone(),
                                    self.session.clone(),
                                    self.config.account_index,
                                );
                                self.radio =
                                    Radio::new(client.clone(), self.config.radio_servers.clone())
                                        .with_cache(self.paths.cache.join("radio"));
                                self.images =
                                    ImageCache::new(client, self.paths.cache.join("images"));
                            }
                            Err(e) => self.status = e.to_string(),
                        }
                        if network_changed || radio_changed {
                            for tab in 1..=3 {
                                if !network_changed && tab != 3 {
                                    continue;
                                }
                                let view = &mut self.tabs[tab];
                                view.cancel.cancel();
                                view.generation = view.generation.wrapping_add(1);
                                view.loading = false;
                                if view.error.is_some() && view.load.is_none() {
                                    view.load = view.request.clone().map(|request| FeedLoad {
                                        request,
                                        append: false,
                                        preserve_selection: true,
                                        failures: 0,
                                        retry_at: None,
                                    });
                                }
                                view.error = None;
                                // Preserve the loaded catalog/selection and pending
                                // continuation. Reject completions from the old route.
                                if let Some(load) = &mut view.load {
                                    load.retry_at = Some(Instant::now());
                                }
                            }
                            if network_changed {
                                // Background watches hold cloned providers too. Cancel
                                // their old route and reject even already-queued results,
                                // retaining baselines and enabled/paused intent.
                                for (_, token) in self.watch_pending.values() {
                                    token.cancel();
                                }
                                self.watch_pending.clear();
                                for watch in &mut self.snapshot.watches {
                                    watch.generation = watch.generation.wrapping_add(1);
                                }
                                self.dirty = true;
                                self.card_cancel.cancel();
                                self.card_cancel = CancellationToken::new();
                                self.card_generation = self.card_generation.wrapping_add(1);
                                self.card_requests.clear();
                                self.card_failures.clear();
                                self.card_pending = 0;
                            }
                            if !self.demo && !self.offline && self.tabs[self.tab].load.is_some() {
                                self.start_browse(self.tab);
                            }
                        }
                        self.status =
                            "Settings saved · changes apply to new downloads and playback".into();
                        self.selection_changed();
                        if account_changed {
                            self.reset_account();
                        }
                    }
                }
                self.continue_disconnect();
            }
            UiEvent::Auth { operation, result } => {
                if !self.finish_operation(&operation) {
                    return;
                }
                match result {
                    Ok(s) => {
                        self.status = format!("Connected as {}", s.account);
                        self.session = Some(Arc::new(s));
                        self.reset_account();
                    }
                    Err(e) => {
                        self.status = format!(
                            "Sign-in failed: {}",
                            ytui_services::process::diagnostic(&e.to_string())
                        )
                    }
                }
                self.continue_disconnect();
            }
            UiEvent::Disconnected { operation, result } => {
                if !self.finish_operation(&operation) {
                    return;
                }
                self.session = None;
                self.reset_account();
                match result {
                    Ok(()) => {
                        self.status = "Disconnected · browser cookies were left intact".into();
                    }
                    Err(e) => {
                        self.status = format!(
                            "Disconnected in memory; storage cleanup failed: {} · fix storage/unlock keyring and retry before restarting",
                            ytui_services::process::diagnostic(&e.to_string())
                        )
                    }
                }
                self.continue_disconnect();
            }
            UiEvent::Clipboard { generation, result } => {
                self.clipboard_pending = false;
                if generation != self.clipboard_generation || !self.config.clipboard_watcher {
                    return;
                }
                match result {
                    Ok(text) => {
                        if let Some(url) = self.clipboard_monitor.observe(&text) {
                            self.clipboard_url = Some(url);
                            self.status =
                                "YouTube URL detected · :clipboard reviews download options".into();
                        }
                        self.clipboard_due = Instant::now() + Duration::from_secs(2);
                    }
                    Err(_) => {
                        if !self.clipboard_warned {
                            self.status = "Clipboard watcher unavailable · install wl-clipboard or xclip/xsel; retrying in 30s".into();
                            self.clipboard_warned = true;
                        }
                        self.clipboard_due = Instant::now() + Duration::from_secs(30);
                    }
                }
            }
            UiEvent::Watch {
                id,
                generation,
                result,
            } => {
                if self
                    .watch_pending
                    .get(&id)
                    .is_some_and(|(g, _)| *g == generation)
                {
                    self.watch_pending.remove(&id);
                }
                let Some(watch) = self
                    .snapshot
                    .watches
                    .iter_mut()
                    .find(|w| w.id == id && w.generation == generation && w.enabled)
                else {
                    return;
                };
                let options = watch.options.clone();
                let baseline = watch.seen.is_none();
                match result {
                    Ok(items) => {
                        let added = watch.observe(items, schedule::unix_now());
                        let mut count = 0;
                        for media in added {
                            let before = self.snapshot.downloads.jobs.len();
                            self.enqueue_download(media, options.clone());
                            if self.snapshot.downloads.jobs.len() > before {
                                self.snapshot.downloads.jobs.last_mut().unwrap().group_id =
                                    Some(format!("watch:{id}"));
                                count += 1;
                            }
                        }
                        self.status = if baseline {
                            "Watch baseline saved · only future additions will download".into()
                        } else {
                            format!("Watch checked · {count} new downloads queued")
                        };
                    }
                    Err(e) => {
                        let message = ytui_services::process::diagnostic(&e.to_string());
                        let now = schedule::unix_now();
                        watch.failed(message.clone(), now);
                        if let Some(retry) = e.downcast_ref::<ytui_services::browse::FeedRetry>() {
                            let seconds = retry
                                .after
                                .as_secs()
                                .saturating_add(u64::from(retry.after.subsec_nanos() > 0));
                            watch.next_check = watch.next_check.max(now.saturating_add(seconds));
                        }
                        self.status = format!("Watch check failed · {message}");
                    }
                }
                self.dirty = true;
            }
            UiEvent::Import(result) => match result {
                Ok(jobs) => {
                    let count =
                        ytui_services::queue_import::append(&mut self.snapshot.downloads, jobs);
                    self.status = format!("Imported {count} queued items");
                    self.dirty = true;
                }
                Err(e) => self.status = ytui_services::process::diagnostic(&e.to_string()),
            },
            UiEvent::Text { id, result } => {
                if let Some(Modal::PendingText { id: current, title }) = &self.modal
                    && *current == id
                {
                    self.modal = Some(Modal::Text {
                        title: title.clone(),
                        lines: result.unwrap_or_else(|error| {
                            vec![ytui_services::process::diagnostic(&error.to_string())]
                        }),
                        offset: 0,
                    });
                }
            }
            UiEvent::Notification(result) => {
                self.notification_pending = self.notification_pending.saturating_sub(1);
                if result.is_err() && !self.notification_warned {
                    self.notification_warned = true;
                    self.status =
                        "Desktop notifications unavailable · download result is saved in Downloads"
                            .into();
                }
            }
            UiEvent::Status(message) => {
                if !message.is_empty() {
                    self.status = message;
                }
            }
        }
        self.redraw = true;
    }
    fn reset_account(&mut self) {
        self.player_session_pending = true;
        self.pending_auto_play = None;
        self.collection_cancel.cancel();
        self.collection_generation = self.collection_generation.wrapping_add(1);
        self.collection_pending = false;
        for job in &mut self.snapshot.downloads.jobs {
            if job.state.active() {
                if let Some(task) = self.running.get(&job.id) {
                    task.cancel.cancel();
                }
                job.invalidate(JobState::Paused);
            }
        }
        for tab in 1..=2 {
            self.tabs[tab].cancel.cancel();
            self.tabs[tab].generation += 1;
            self.tabs[tab].page = Page::default();
            self.tabs[tab].history.clear();
            self.tabs[tab].request = None;
            self.tabs[tab].loading = false;
            self.tabs[tab].load = None;
            self.tabs[tab].error = None;
            self.tabs[tab].empty_pages = 0;
            self.tabs[tab].seen_tokens.clear();
        }
        if let Ok(client) = ytui_services::http_client(self.config.proxy.as_deref()) {
            self.browser = Browser::new(client, self.session.clone(), self.config.account_index);
        }
        self.sync_player_session();
        self.snapshot.playback = PlaybackQueue::default();
        self.snapshot.positions.clear_remote();
        for watch in &mut self.snapshot.watches {
            watch.enabled = false;
            watch.generation += 1;
            watch.seen = None;
            watch.error =
                Some("Paused after account change; resume to establish a fresh baseline".into());
        }
        for (_, token) in self.watch_pending.values() {
            token.cancel();
        }
        self.watch_pending.clear();
        self.dirty = true;
        self.selection_changed();
        if self.tab == 1 || self.tab == 2 {
            self.navigate(self.tab, BrowseRequest::Home, false);
        }
    }
    pub fn save_config(&mut self, mut c: Config) {
        if self.pending_operation.is_some() || self.disconnect_pending {
            self.status = "Account/settings operation in progress · draft retained".into();
            self.modal = Some(Modal::Form(Box::new(Form::settings(c))));
            return;
        }
        c.offline = self.offline;
        if self.demo {
            self.config = c;
            if let Ok(p) = theme::builtin(&self.config.theme) {
                self.palette = p;
            }
            self.selection_changed();
            self.status = "Demo theme/settings applied temporarily".into();
            self.redraw = true;
            return;
        }
        let paths = self.paths.clone();
        let account_changed = self.config.account_index != c.account_index;
        let storage_changed = self.config.auth_store != c.auth_store;
        let current = self.session.clone();
        let had_session = current.is_some();
        let operation = self.operations.config(account_changed || storage_changed);
        self.pending_operation = Some((operation.clone(), "Saving settings…".into()));
        let operations = self.operations.clone();
        self.tasks.spawn(async move {
            let mut account = None;
            let result = async {
                ytui_services::http_client(c.proxy.as_deref())?;
                if (account_changed || storage_changed)
                    && let Some(session) = current
                {
                    let mut candidate = (*session).clone();
                    if account_changed {
                        let browser = Browser::new(
                            ytui_services::http_client(c.proxy.as_deref())?,
                            Some(session),
                            c.account_index,
                        );
                        candidate.account = operation.prepare(browser.validate()).await?;
                    }
                    account = Some(candidate);
                }
                let saved = c.clone();
                let session = account.clone();
                operations
                    .commit(operation.clone(), move || {
                        if let Some(session) = session {
                            SessionStore::new(&paths, &saved.auth_store).save(&session)?;
                        } else if storage_changed && !had_session {
                            SessionStore::new(&paths, &saved.auth_store).suspend_auto_load()?;
                        }
                        paths.save_config(&saved)
                    })
                    .await?;
                Ok(())
            }
            .await;
            UiEvent::Config {
                operation,
                config: Box::new(c),
                account,
                result,
            }
        });
    }
    pub fn key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        self.redraw = true;
        if let Some(mut input) = self.input.take() {
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let text = input.editor.text.trim().to_owned();
                    self.search(&text);
                }
                _ => {
                    input.editor.key(key);
                    self.input = Some(input);
                }
            }
            return;
        }
        if self.modal.is_some() {
            self.modal_key(key);
            return;
        }
        let name = key_name(key);
        let action = self
            .config
            .keybindings
            .get(&name)
            .and_then(|v| Action::parse(v))
            .or_else(|| {
                REGISTRY
                    .iter()
                    .find(|(k, action, _, _)| {
                        *k == name && !self.config.keybindings.values().any(|v| v == action.name())
                    })
                    .map(|(_, a, _, _)| *a)
            })
            .or(match name.as_str() {
                ":" => Some(Action::Command),
                "j" => Some(Action::Down),
                "k" => Some(Action::Up),
                _ => None,
            });
        if let Some(action) = action {
            self.action(action);
        } else if let KeyCode::Char(c @ '1'..='4') = key.code {
            self.change_tab(c as usize - '1' as usize);
        }
    }
    fn modal_key(&mut self, key: KeyEvent) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            return;
        }
        match &mut modal {
            Modal::PendingText { .. } => self.modal = Some(modal),
            Modal::Lyrics(view) => {
                if !view.loading {
                    let limit = if view.opened {
                        view.tracks
                            .get(view.selected)
                            .map_or(0, |track| track.lines.len().saturating_sub(1))
                    } else {
                        view.tracks.len().saturating_sub(1)
                    };
                    match key.code {
                        KeyCode::Enter if !view.opened && !view.tracks.is_empty() => {
                            view.opened = true;
                            view.offset = 0;
                            view.follow = true;
                        }
                        KeyCode::Backspace if view.opened && view.source == "LRCLIB" => {
                            view.opened = false
                        }
                        KeyCode::Char('f') | KeyCode::Home if view.opened => {
                            view.follow = true;
                            view.offset = 0;
                        }
                        code @ (KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                        | KeyCode::End) => {
                            let offset = if view.opened {
                                &mut view.offset
                            } else {
                                &mut view.selected
                            };
                            *offset = match code {
                                KeyCode::Up => offset.saturating_sub(1),
                                KeyCode::Down => (*offset + 1).min(limit),
                                KeyCode::PageUp => offset.saturating_sub(10),
                                KeyCode::PageDown => (*offset + 10).min(limit),
                                _ => limit,
                            };
                            view.follow = false;
                        }
                        _ => {}
                    }
                }
                self.modal = Some(modal);
            }
            Modal::Actions { selected, query } => {
                let matches =
                    crate::actions::matching_localized(&query.text, &self.config.language);
                match key.code {
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => {
                        *selected = (*selected + 1).min(matches.len().saturating_sub(1))
                    }
                    KeyCode::PageUp => *selected = selected.saturating_sub(10),
                    KeyCode::PageDown => {
                        *selected = (*selected + 10).min(matches.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(index) = matches.get(*selected) {
                            self.open_action(*index);
                        }
                        return;
                    }
                    _ => {
                        query.key(key);
                        *selected = 0;
                    }
                }
                self.modal = Some(modal);
            }
            Modal::Categories(selected) => {
                let categories = &self.tabs[self.tab].page.categories;
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(categories.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(category) = categories.get(*selected).cloned() {
                            self.navigate(self.tab, category.request, false);
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Feeds(index) => {
                let choices = self.feed_choices();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *index = (*index + 1).min(choices.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some((_, _, command)) = choices.get(*index)
                            && let Err(error) = self.command(command)
                        {
                            self.status = error.to_string();
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Text { lines, offset, .. } => {
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *offset = (*offset + 1).min(lines.len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => *offset = offset.saturating_sub(1),
                    KeyCode::PageDown => {
                        *offset = (*offset + 10).min(lines.len().saturating_sub(1))
                    }
                    KeyCode::PageUp => *offset = offset.saturating_sub(10),
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Help(offset) => {
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *offset = (*offset + 1).min(REGISTRY.len())
                    }
                    KeyCode::Up | KeyCode::Char('k') => *offset = offset.saturating_sub(1),
                    KeyCode::PageDown => *offset = (*offset + 10).min(REGISTRY.len()),
                    KeyCode::PageUp => *offset = offset.saturating_sub(10),
                    KeyCode::Home => *offset = 0,
                    KeyCode::End => *offset = REGISTRY.len(),
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::History(view) => {
                let rows = view.rows(&self.snapshot.downloads.jobs);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        view.selected = view.selected.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        view.selected = (view.selected + 1).min(rows.len().saturating_sub(1))
                    }
                    KeyCode::Char('f') => {
                        view.filter = view.filter.next();
                        view.selected = 0;
                    }
                    KeyCode::Char('r') => {
                        if let Some(index) = rows.get(view.selected).map(|(index, _)| *index) {
                            let job = &mut self.snapshot.downloads.jobs[index];
                            if matches!(
                                job.state,
                                JobState::Error | JobState::Cancelled | JobState::Paused
                            ) {
                                job.invalidate(JobState::Queued);
                                job.error = None;
                                job.attempt = 0;
                                self.dirty = true;
                                self.status =
                                    "Selected entry queued for retry · completed files retained"
                                        .into();
                                view.selected = 0;
                            }
                        }
                    }
                    KeyCode::Char('o') => {
                        if let Some((_, job)) = rows.get(view.selected) {
                            self.modal = Some(Modal::Outputs {
                                job: job.id,
                                selected: 0,
                            });
                            return;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some((index, _)) = rows.get(view.selected) {
                            self.tab = 0;
                            self.download_selected = *index;
                            self.selection_changed();
                        }
                        return;
                    }
                    KeyCode::Char('g') => {
                        if let Some((_, job)) = rows.get(view.selected)
                            && let Some(group) = &job.group_id
                        {
                            self.modal = Some(Modal::History(history::View {
                                group: Some(group.clone()),
                                ..Default::default()
                            }));
                            return;
                        }
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Groups(index) => {
                let groups = self.download_groups();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *index = (*index + 1).min(groups.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some((id, _, _)) = groups.get(*index) {
                            self.modal = Some(Modal::History(history::View {
                                group: Some(id.clone()),
                                ..Default::default()
                            }));
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Themes(index) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    *index = index.saturating_sub(1);
                    self.modal = Some(modal);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = (*index + 1).min(theme::THEMES.len() - 1);
                    self.modal = Some(modal);
                }
                KeyCode::Enter => {
                    let mut c = self.config.clone();
                    c.theme = theme::THEMES[*index].into();
                    self.save_config(c);
                }
                _ => self.modal = Some(modal),
            },
            Modal::Watches(index) => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *index = (*index + 1).min(self.snapshot.watches.len().saturating_sub(1))
                    }
                    KeyCode::Char(' ') => {
                        if let Some(watch) = self.snapshot.watches.get(*index) {
                            self.toggle_watch(watch.id, !watch.enabled);
                        }
                    }
                    KeyCode::Delete => {
                        if let Some(watch) = self.snapshot.watches.get(*index) {
                            let id = watch.id;
                            self.toggle_watch(id, false);
                            self.snapshot.watches.retain(|w| w.id != id);
                            self.dirty = true;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(watch) = self.snapshot.watches.get(*index) {
                            let tab = if watch.music { 2 } else { 1 };
                            let request = watch.request.clone();
                            self.tab = tab;
                            self.navigate(tab, request, false);
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Resume(index) => {
                let positions = self.resume_positions();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *index = (*index + 1).min(positions.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(position) = positions.get(*index) {
                            self.resume_position((*position).clone());
                        }
                        return;
                    }
                    KeyCode::Delete => {
                        if let Some(position) = positions.get(*index) {
                            let url = position.media.url.clone();
                            self.snapshot
                                .positions
                                .entries
                                .retain(|p| p.media.url != url);
                            self.dirty = true;
                        }
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Destinations(selected) => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1)
                            .min(self.snapshot.recent_destinations.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(path) =
                            self.snapshot.recent_destinations.get(*selected).cloned()
                            && let Some(media) = self.selected()
                        {
                            let mut options = self.config.download.clone();
                            options.audio = self.selection_is_audio(&media);
                            if options.audio {
                                options.audio_directory = path;
                            } else {
                                options.directory = path;
                            }
                            let mut form = Form::download(media, options);
                            form.select_section(1);
                            form.selected = 4;
                            self.modal = Some(Modal::Form(Box::new(form)));
                            return;
                        }
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::PlayerSettings { section, selected } => {
                match key.code {
                    KeyCode::Left => {
                        *section = (*section + 2) % 3;
                        *selected = 0;
                    }
                    KeyCode::Right | KeyCode::Tab => {
                        *section = (*section + 1) % 3;
                        *selected = 0;
                    }
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1)
                            .min(self.playback_choices(*section).len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some((_, command)) = self.playback_choices(*section).get(*selected) {
                            self.status = if self.player.try_send(command.clone()).is_ok() {
                                "Applying playback selection · position/pause retained"
                            } else {
                                "Player busy · try again"
                            }
                            .into();
                            return;
                        }
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Account { profiles, selected } => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(profiles.len())
                    }
                    KeyCode::Enter => {
                        let mut form = Form::auth();
                        if let Some(profile) = selected
                            .checked_sub(1)
                            .and_then(|index| profiles.get(index))
                        {
                            form.fields[0].value = Editor::new(profile.browser.clone());
                            form.fields[1].value = Editor::new(profile.path.display().to_string());
                            form.selected = 1;
                        }
                        self.modal = Some(Modal::Form(Box::new(form)));
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            Modal::Outputs { job, selected } => {
                if let Some(download) = self.snapshot.downloads.jobs.iter().find(|j| j.id == *job) {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                        KeyCode::Down | KeyCode::Char('j') => {
                            *selected =
                                (*selected + 1).min(download.outputs.len().saturating_sub(1))
                        }
                        KeyCode::Char('o') => {
                            if let Some(media) = download.outputs.get(*selected).and_then(|path| {
                                ytui_services::library::completed_file(download, path)
                            }) {
                                let path = PathBuf::from(media.url);
                                self.tasks.spawn(async move {
                                    UiEvent::Status(
                                        match ytui_services::desktop::open_file(&path).await {
                                            Ok(()) => {
                                                "Open with request sent to the desktop portal"
                                                    .into()
                                            }
                                            Err(error) => format!(
                                                "Desktop open unavailable: {}",
                                                ytui_services::process::diagnostic(
                                                    &error.to_string()
                                                )
                                            ),
                                        },
                                    )
                                });
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(media) = download.outputs.get(*selected).and_then(|path| {
                                ytui_services::library::completed_file(download, path)
                            }) {
                                self.play(media);
                                return;
                            }
                            self.status =
                                "This file is missing or is a subtitle/image sidecar".into();
                        }
                        _ => {}
                    }
                }
                self.modal = Some(modal);
            }
            Modal::Queue(index) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    *index = index.saturating_sub(1);
                    self.modal = Some(modal);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = (*index + 1).min(self.snapshot.playback.items.len().saturating_sub(1));
                    self.modal = Some(modal);
                }
                KeyCode::Delete => {
                    self.snapshot.playback.remove(*index);
                    self.dirty = true;
                    self.modal = Some(modal);
                }
                KeyCode::Enter => {
                    if let Some(m) = self.snapshot.playback.items.get(*index).cloned()
                        && self.play_existing(m)
                    {
                        self.snapshot.playback.current = Some(*index);
                        self.dirty = true;
                    }
                }
                _ => self.modal = Some(modal),
            },
            Modal::Form(form) => {
                let settings = form.is_grouped();
                if settings && key.code == KeyCode::F(6) {
                    form.sections_focused = !form.sections_focused;
                    self.modal = Some(modal);
                    return;
                }
                if settings
                    && (matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
                        || key.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(key.code, KeyCode::Left | KeyCode::Right))
                {
                    let delta = if matches!(key.code, KeyCode::PageUp | KeyCode::Left) {
                        -1
                    } else {
                        1
                    };
                    let next = (form.section as isize + delta)
                        .rem_euclid(form.sections().len() as isize)
                        as usize;
                    form.select_section(next);
                    self.modal = Some(modal);
                    return;
                }
                if settings && form.sections_focused {
                    match key.code {
                        KeyCode::Up => form.select_section(form.section.saturating_sub(1)),
                        KeyCode::Down => form.select_section(form.section + 1),
                        KeyCode::Tab | KeyCode::Right | KeyCode::Enter => {
                            form.sections_focused = false
                        }
                        _ => {}
                    }
                    self.modal = Some(modal);
                    return;
                }
                match key.code {
                    KeyCode::Up | KeyCode::BackTab => form.move_field(-1),
                    KeyCode::Down | KeyCode::Tab => form.move_field(1),
                    KeyCode::Char(' ') if form.fields[form.selected].toggle => form.toggle(),
                    KeyCode::Left | KeyCode::Right
                        if !form.fields[form.selected].choices.is_empty() =>
                    {
                        form.cycle_choice(if key.code == KeyCode::Left { -1 } else { 1 });
                    }
                    KeyCode::Enter => {
                        if matches!(
                            form.kind,
                            FormKind::Settings(_) | FormKind::Keys(_) | FormKind::Auth
                        ) && (self.pending_operation.is_some() || self.disconnect_pending)
                        {
                            form.error = Some("Account/settings operation in progress. Cancel it from Actions or wait; this draft is retained.".into());
                            self.modal = Some(modal);
                            return;
                        }
                        let outcome = match &form.kind {
                            FormKind::Download(_, _) => form.download_values().map(|(m, o)| {
                                self.enqueue_download(m, o);
                            }),
                            FormKind::Keys(_) => form.key_values().map(|c| self.save_config(c)),
                            FormKind::Settings(_) => {
                                form.config_values().map(|c| self.save_config(c))
                            }
                            FormKind::Action {
                                command, media_url, ..
                            } => {
                                let lyrics = matches!(*command, "lyrics-find" | "lyrics-local");
                                let command = format!(
                                    "{} {}",
                                    command,
                                    form.fields
                                        .iter()
                                        .map(|f| shell_words::quote(f.value.text.trim()))
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                );
                                self.command(&command).map(|()| {
                                    if lyrics && let Some(Modal::Lyrics(view)) = &mut self.modal {
                                        // The form may stay open while automatic playback
                                        // advances. Keep the original selection's identity.
                                        view.media_url = media_url.clone();
                                    }
                                })
                            }
                            FormKind::Auth => {
                                let browser = form.value(0).to_owned();
                                let profile = form.value(1).to_owned();
                                let file = form.value(2).to_owned();
                                self.import_auth(browser, profile, file);
                                Ok(())
                            }
                        };
                        if let Err(e) = outcome {
                            form.error = Some(e.to_string());
                            self.modal = Some(modal);
                        }
                        return;
                    }
                    _ => {
                        if !form.fields[form.selected].toggle {
                            form.fields[form.selected].value.key(key);
                        }
                    }
                }
                self.modal = Some(modal);
            }
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(input) = &mut self.input {
            input.editor.insert(text);
        } else if let Some(Modal::Actions { query, selected }) = &mut self.modal {
            query.insert(text);
            *selected = 0;
        } else if let Some(Modal::Form(form)) = &mut self.modal {
            if !form.fields[form.selected].toggle {
                form.fields[form.selected].value.insert(text);
            }
        } else {
            let mut editor = Editor::default();
            editor.insert(text);
            self.input = Some(Input { editor });
        }
        self.redraw = true;
    }
    pub fn change_tab(&mut self, tab: usize) {
        self.tab = tab % 4;
        self.feeds_focused = false;
        self.feed_selected = 0;
        self.selection_changed();
        if self.tab > 0 && self.tabs[self.tab].request.is_none() && !self.demo {
            self.navigate(
                self.tab,
                if self.tab == 3 {
                    BrowseRequest::Cliamp
                } else {
                    BrowseRequest::Home
                },
                false,
            );
        }
        self.redraw = true;
    }
    pub fn move_selection(&mut self, delta: isize) {
        if self.tab == 0 {
            self.download_selected = self
                .download_selected
                .saturating_add_signed(delta)
                .min(self.snapshot.downloads.jobs.len().saturating_sub(1));
        } else {
            let view = &mut self.tabs[self.tab];
            view.selected = view
                .selected
                .saturating_add_signed(delta)
                .min(view.page.items.len().saturating_sub(1));
        }
        // Vinyl follows the player, so catalog navigation must retain its
        // decoded cover and any in-flight playing-art request.
        if !self.vinyl_enabled() {
            self.selection_changed();
        }
        self.redraw = true;
    }
    pub fn mouse(&mut self, mouse: MouseEvent) {
        if let Some(Modal::Form(form)) = &mut self.modal
            && form.is_grouped()
        {
            let hit = self
                .hit_targets
                .iter()
                .rev()
                .find(|(rect, _)| rect.contains(Position::new(mouse.column, mouse.row)))
                .map(|(_, hit)| *hit);
            match (mouse.kind, hit) {
                (MouseEventKind::Down(_), Some(HitTarget::SettingSection(section))) => {
                    form.select_section(section);
                    form.sections_focused = false;
                }
                (MouseEventKind::Down(_), Some(HitTarget::SettingField(index))) => {
                    form.selected = index;
                    form.sections_focused = false;
                    form.cycle_choice(1);
                }
                (MouseEventKind::Down(_), Some(HitTarget::SettingsSave)) => {
                    form.sections_focused = false;
                    self.modal_key(KeyCode::Enter.into());
                }
                (MouseEventKind::Down(_), Some(HitTarget::SettingsCancel)) => {
                    self.modal_key(KeyCode::Esc.into())
                }
                (MouseEventKind::ScrollDown, _) => form.move_field(1),
                (MouseEventKind::ScrollUp, _) => form.move_field(-1),
                _ => {}
            }
            self.redraw = true;
            return;
        }
        if self.modal.is_some() {
            let hit = self
                .hit_targets
                .iter()
                .rev()
                .find(|(rect, _)| rect.contains(Position::new(mouse.column, mouse.row)))
                .map(|(_, hit)| *hit);
            if let Some(HitTarget::MenuItem(index)) = hit {
                match &mut self.modal {
                    Some(Modal::Actions { selected, .. }) | Some(Modal::Categories(selected)) => {
                        *selected = index;
                        self.modal_key(KeyCode::Enter.into());
                    }
                    _ => {}
                }
            }
            self.redraw = true;
            return;
        }
        if self.input.is_some() {
            return;
        }
        let hit = self
            .hit_targets
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(Position::new(mouse.column, mouse.row)))
            .map(|(_, hit)| *hit);
        match (mouse.kind, hit) {
            (MouseEventKind::Down(_), Some(HitTarget::Action(action))) => {
                if action == Action::Pause {
                    let _ = self.player.try_send(PlayerCommand::TogglePause);
                } else {
                    self.action(action);
                }
            }
            (MouseEventKind::Down(_), Some(HitTarget::Category(index))) => {
                if let Some(category) = self.tabs[self.tab].page.categories.get(index).cloned() {
                    self.navigate(self.tab, category.request, false);
                }
            }
            (MouseEventKind::Down(_), Some(HitTarget::Timeline)) => {
                if let Some((rect, _)) = self
                    .hit_targets
                    .iter()
                    .find(|(_, target)| *target == HitTarget::Timeline)
                    && let Some(duration) = self.player_status.duration
                {
                    let fraction = mouse.column.saturating_sub(rect.x) as f64
                        / rect.width.saturating_sub(1).max(1) as f64;
                    let _ = self
                        .player
                        .try_send(PlayerCommand::SeekTo(fraction * duration));
                }
            }
            (MouseEventKind::Down(_), Some(HitTarget::Tab(tab))) => self.change_tab(tab),
            (MouseEventKind::Down(_), Some(HitTarget::Feed(index))) => {
                self.feed_selected = index;
                self.feeds_focused = true;
            }
            (MouseEventKind::Down(_), Some(HitTarget::Item(index))) => {
                self.feeds_focused = false;
                let selected = if self.tab == 0 {
                    self.download_selected
                } else {
                    self.tabs[self.tab].selected
                };
                self.move_selection(index as isize - selected as isize);
            }
            (MouseEventKind::ScrollDown | MouseEventKind::ScrollUp, Some(target)) => {
                let delta = if mouse.kind == MouseEventKind::ScrollDown {
                    1
                } else {
                    -1
                };
                if matches!(target, HitTarget::Feed(_) | HitTarget::Feeds) {
                    self.feeds_focused = true;
                    self.feed_selected = self
                        .feed_selected
                        .saturating_add_signed(delta)
                        .min(self.feed_choices().len().saturating_sub(1));
                } else if matches!(target, HitTarget::Item(_) | HitTarget::Catalog) {
                    self.feeds_focused = false;
                    self.move_selection(delta);
                }
            }
            _ => {}
        }
        self.redraw = true;
    }
    fn selection_changed(&mut self) {
        self.image_cancel.cancel();
        self.image_cancel = CancellationToken::new();
        self.image = None;
        self.image_key = None;
        self.image_error = None;
        self.image_failures = 0;
        self.image_generation += 1;
        self.image_due = Instant::now();
        self.prefetch_images();
    }
    fn thumbnail_size(&self) -> (u32, u32) {
        let (w, h) = self.picker.font_size();
        (
            (self.image_area.width as u32 * w as u32).clamp(1, 1280),
            (self.image_area.height as u32 * h as u32).clamp(1, 720),
        )
    }
    fn prefetch_images(&mut self) {
        if self.demo
            || self.offline
            || self.tab == 0
            || !self.config.thumbnails
            || self.config.image_protocol == "none"
        {
            return;
        }
        let (width, height) = self
            .card_areas
            .first()
            .map(|(_, area)| {
                let (fw, fh) = self.picker.font_size();
                (
                    (area.width as u32 * fw as u32).clamp(1, 1280),
                    (area.height as u32 * fh as u32).clamp(1, 720),
                )
            })
            .unwrap_or_else(|| self.thumbnail_size());
        if width <= 1 || height <= 1 {
            return;
        }
        let view = &self.tabs[self.tab];
        let urls = view
            .page
            .items
            .iter()
            .skip(view.selected.saturating_sub(2))
            .take(10)
            .filter(|m| !m.thumbnail.is_empty())
            .map(|m| ytui_services::images::thumbnail_url(m, width, height))
            .collect::<Vec<_>>();
        let cache = self.images.clone();
        let cancel = self.image_cancel.clone();
        self.tasks.spawn(async move {
            use futures_util::{StreamExt, stream};
            let work = stream::iter(urls).for_each_concurrent(3, |url| {
                let cache = cache.clone();
                async move {
                    let _ = cache.load_sized(&url, width, height).await;
                }
            });
            tokio::select! {_=cancel.cancelled()=>{},_=work=>{}};
            UiEvent::Prefetched
        });
    }
    fn recover_feed(&mut self) {
        if self.demo || self.offline || !matches!(self.tab, 1..=3) {
            return;
        }
        let view = &self.tabs[self.tab];
        if !view.loading
            && view
                .load
                .as_ref()
                .and_then(|load| load.retry_at)
                .is_some_and(|at| at <= Instant::now())
        {
            self.start_browse(self.tab);
        }
    }
    fn prefetch_page(&mut self) {
        if self.tab == 0 || self.demo || self.offline {
            return;
        }
        let view = &self.tabs[self.tab];
        if !view.loading
            && view.error.is_none()
            && view.load.is_none()
            && (view.selected.saturating_add(12) >= view.page.items.len()
                || self
                    .hit_targets
                    .iter()
                    .filter_map(|(_, hit)| {
                        if let HitTarget::Item(index) = hit {
                            Some(index.saturating_add(12))
                        } else {
                            None
                        }
                    })
                    .max()
                    .unwrap_or(0)
                    >= view.page.items.len())
            && view.page.continuation.is_some()
        {
            self.action(Action::NextPage);
        }
    }
    pub fn card_key(&self, media: &Media, area: Rect) -> String {
        format!(
            "{}:{:?}:{}:{}:{}",
            self.palette.name,
            self.picker.protocol_type(),
            media.thumbnail,
            area.width,
            area.height
        )
    }
    fn refresh_cards(&mut self) {
        if self.offline
            || self.modal.is_some()
            || !self.config.thumbnails
            || self.config.image_protocol == "none"
        {
            return;
        }
        let (fw, fh) = self.picker.font_size();
        for (index, area) in self.card_areas.clone() {
            if self.card_pending >= 4 {
                break;
            }
            let Some(media) = self.tabs[self.tab].page.items.get(index).cloned() else {
                continue;
            };
            if media.thumbnail.is_empty() {
                continue;
            }
            let key = self.card_key(&media, area);
            if self.card_images.contains_key(&key)
                || self
                    .card_failures
                    .get(&key)
                    .is_some_and(|(_, due)| due.is_none_or(|at| at > Instant::now()))
            {
                continue;
            }
            if !self.card_requests.insert(key.clone()) {
                continue;
            }
            self.card_pending += 1;
            let (width, height) = (
                area.width as u32 * fw as u32,
                area.height as u32 * fh as u32,
            );
            let url = ytui_services::images::thumbnail_url(&media, width, height);
            let cache = self.images.clone();
            let picker = self.picker.clone();
            let demo = self.demo.then(|| self.palette.clone());
            let generation = self.card_generation;
            let cancel = self.card_cancel.clone();
            self.tasks.spawn(async move {
                let result = async {
                    let image = if let Some(palette) = demo {
                        tokio::task::spawn_blocking(move || {
                            demo_art(&palette, index, width, height)
                        })
                        .await?
                    } else {
                        tokio::select! {
                            _ = cancel.cancelled() => anyhow::bail!("Thumbnail request cancelled"),
                            image = cache.load_sized(&url, width, height) => image?,
                        }
                    };
                    tokio::task::spawn_blocking(move || {
                        picker.new_protocol(
                            image,
                            Rect::new(0, 0, area.width, area.height),
                            Resize::Fit(None),
                        )
                    })
                    .await?
                    .map_err(anyhow::Error::from)
                }
                .await;
                UiEvent::CardImage {
                    generation,
                    key,
                    result,
                }
            });
        }
        // Retain failure backoff only for the visible window once the map fills.
        if self.card_failures.len() > 256 {
            let visible = self
                .card_areas
                .iter()
                .filter_map(|(i, rect)| {
                    self.tabs[self.tab]
                        .page
                        .items
                        .get(*i)
                        .map(|media| self.card_key(media, *rect))
                })
                .collect::<std::collections::HashSet<_>>();
            self.card_failures.retain(|key, _| visible.contains(key));
        }
    }
    pub fn image_matches(&self, thumbnail: &str) -> bool {
        self.image_key.as_ref().is_some_and(|(url, w, h)| {
            url == thumbnail && *w == self.image_area.width && *h == self.image_area.height
        })
    }
    pub fn refresh_image(&mut self) {
        self.refresh_cards();
        if self.offline
            || Instant::now() < self.image_due
            || self.config.image_protocol == "none"
            || !self.config.thumbnails
            || self.modal.is_some()
            || self.image_area.width == 0
            || self.image_area.height == 0
        {
            return;
        }
        let media = if self.vinyl_enabled() {
            self.player_status.media.clone()
        } else {
            self.selected()
        };
        let Some(media) = media else {
            return;
        };
        if media.thumbnail.is_empty() {
            return;
        }
        let key = (
            media.thumbnail.clone(),
            self.image_area.width,
            self.image_area.height,
        );
        if self.image_key.as_ref() == Some(&key) {
            return;
        }
        let (width, height) = self.thumbnail_size();
        let url = ytui_services::images::thumbnail_url(&media, width, height);
        self.image_key = Some(key);
        self.image_generation += 1;
        let generation = self.image_generation;
        let cache = self.images.clone();
        let demo_fixture = self.demo.then(|| {
            (
                self.palette.clone(),
                if self.vinyl_enabled() {
                    media
                        .thumbnail
                        .rsplit('/')
                        .next()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0)
                } else {
                    self.tabs[self.tab].selected
                },
            )
        });
        let picker = self.picker.clone();
        let area = Rect::new(0, 0, self.image_area.width, self.image_area.height);
        let cancel = self.image_cancel.clone();
        self.tasks.spawn(async move {
            let work = async {
                let image = if let Some((palette,index)) = demo_fixture { tokio::task::spawn_blocking(move || demo_art(&palette,index,width,height)).await? } else { cache.load_sized(&url,width,height).await? };
                tokio::task::spawn_blocking(move || {
                    picker.new_protocol(image, area, Resize::Fit(None))
                })
                .await?
                .map_err(anyhow::Error::from)
            }
            ;
            let result=tokio::select!{_=cancel.cancelled()=>Err(anyhow::anyhow!("Image request cancelled")),r=work=>r};
            UiEvent::Image { generation, result }
        });
    }
    pub fn vinyl_enabled(&self) -> bool {
        matches!(self.tab, 2 | 3) && self.config.music_player_style != "classic"
    }
    pub fn action(&mut self, action: Action) {
        if self.feeds_focused {
            match action {
                Action::Up => self.feed_selected = self.feed_selected.saturating_sub(1),
                Action::Down => {
                    self.feed_selected =
                        (self.feed_selected + 1).min(self.feed_choices().len().saturating_sub(1))
                }
                Action::Open => {
                    let command = self
                        .feed_choices()
                        .get(self.feed_selected)
                        .map(|(_, _, command)| *command);
                    self.feeds_focused = false;
                    if let Some(command) = command
                        && let Err(error) = self.command(command)
                    {
                        self.status = error.to_string();
                    }
                }
                Action::Back | Action::Feeds => self.feeds_focused = false,
                _ => {
                    self.feeds_focused = false;
                    self.action(action);
                }
            }
            self.redraw = true;
            return;
        }
        match action {
            Action::Feeds => {
                if self.sidebar_visible {
                    self.feeds_focused = true;
                } else {
                    self.modal = Some(Modal::Feeds(0));
                }
            }
            Action::Resume => {
                if let Some(media) = self.selected_for_resume()
                    && let Some(position) = self.snapshot.positions.for_media(&media).cloned()
                {
                    self.resume_position(position);
                } else {
                    self.modal = Some(Modal::Resume(0));
                }
            }
            Action::Fallback => {
                if self.tab != 0 && self.enforce_sleep_timer(Instant::now()) {
                    return;
                }
                if self.player_session_pending {
                    self.status = "Updating player account · retry playback shortly".into();
                    return;
                }
                if self.offline {
                    self.status = "Explicit extraction fallback requires online mode".into();
                    return;
                }
                if self.demo {
                    self.status = "Fallback playback is disabled in demo mode".into();
                    return;
                }
                if self.tab == 0 {
                    if let Some(job) = self.snapshot.downloads.jobs.get_mut(self.download_selected)
                        && !job.state.active()
                    {
                        job.options.backend = Some("yt-dlp".into());
                        job.invalidate(JobState::Queued);
                        job.error = None;
                        job.attempt = 0;
                        self.dirty = true;
                        self.status = "Explicit yt-dlp fallback selected for this download".into();
                    }
                } else if let Some(mut media) = self.selected()
                    && !media.collection()
                {
                    if media.kind == MediaKind::Video && self.selection_is_audio(&media) {
                        media.kind = MediaKind::Song;
                    }
                    let audio = self.selection_is_audio(&media);
                    if self
                        .player
                        .try_send(PlayerCommand::PlayFallback {
                            media: media.clone(),
                            audio,
                        })
                        .is_err()
                    {
                        self.status = "Player busy · try again".into();
                        return;
                    }
                    self.snapshot.playback.play(media);
                    self.dirty = true;
                    self.status = "Playing with explicit yt-dlp fallback".into();
                }
            }
            Action::Quit => self.quit = true,
            Action::Help => self.modal = Some(Modal::Help(0)),
            Action::Search => {
                self.input = Some(Input {
                    editor: Editor::default(),
                })
            }
            Action::Command => {
                self.modal = Some(Modal::Actions {
                    selected: 0,
                    query: Editor::default(),
                })
            }
            Action::PlayerMenu => {
                self.modal = Some(Modal::Actions {
                    selected: 0,
                    query: Editor::new("Player"),
                })
            }
            Action::MusicPlayerStyle => {
                if matches!(self.tab, 2 | 3) {
                    let mut config = self.config.clone();
                    let styles = ytui_core::MUSIC_PLAYER_STYLES;
                    let index = styles
                        .iter()
                        .position(|s| *s == config.music_player_style)
                        .unwrap_or(0);
                    config.music_player_style = styles[(index + 1) % styles.len()].into();
                    self.save_config(config);
                } else {
                    self.status = "Music player styles are available in Music and Radio".into();
                }
            }
            Action::ToggleHints => {
                let mut config = self.config.clone();
                config.show_hints = !config.show_hints;
                self.config.show_hints = config.show_hints;
                self.save_config(config);
            }
            Action::Categories => self.modal = Some(Modal::Categories(0)),
            Action::Tracks => {
                if let Err(e) = self.command("tracks") {
                    self.status = e.to_string();
                }
            }
            Action::SpeedUp => {
                let _ = self.player.try_send(PlayerCommand::Speed(0.25));
            }
            Action::SpeedDown => {
                let _ = self.player.try_send(PlayerCommand::Speed(-0.25));
            }
            Action::Fullscreen => {
                let _ = self.player.try_send(PlayerCommand::Fullscreen);
            }
            Action::CycleAudio => {
                let _ = self.player.try_send(PlayerCommand::CycleAudio);
            }
            Action::CycleSubtitles => {
                let _ = self.player.try_send(PlayerCommand::CycleSubtitles);
            }
            Action::ToggleSubtitles => {
                let _ = self.player.try_send(PlayerCommand::ToggleSubtitles);
            }
            Action::NextChapter => {
                let _ = self.player.try_send(PlayerCommand::Chapter(1));
            }
            Action::PreviousChapter => {
                let _ = self.player.try_send(PlayerCommand::Chapter(-1));
            }
            Action::SponsorToggle => {
                let _ = self.player.try_send(PlayerCommand::ToggleSponsorBlock);
            }
            Action::SponsorUndo => {
                let _ = self.player.try_send(PlayerCommand::UndoSkip);
            }
            Action::NextTab => self.change_tab((self.tab + 1) % 4),
            Action::PreviousTab => self.change_tab((self.tab + 3) % 4),
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Open => {
                if self.tab == 0 {
                    if let Some(job) = self.snapshot.downloads.jobs.get(self.download_selected) {
                        if job.outputs.len() == 1 && job.state == JobState::Done {
                            if let Some(media) =
                                ytui_services::library::completed_file(job, &job.outputs[0])
                            {
                                self.play(media);
                            }
                        } else {
                            self.modal = Some(Modal::Outputs {
                                job: job.id,
                                selected: 0,
                            });
                        }
                    }
                } else if let Some(media) = self.selected() {
                    if let Some(id) = media.browse_id {
                        self.navigate(self.tab, BrowseRequest::Collection(id), false);
                    } else {
                        self.play(media);
                    }
                }
            }
            Action::Preview => {
                if let Some(media) = self.selected()
                    && !media.collection()
                    && media.kind != MediaKind::Station
                    && !self.demo
                    && !self.player_session_pending
                    && (!self.offline || http_url(&media.url).is_err())
                    && !self.enforce_sleep_timer(Instant::now())
                {
                    let _ = self.player.try_send(PlayerCommand::Preview(media));
                }
            }
            Action::Download => {
                if let Some(media) = self.selected() {
                    let mut o = self.config.download.clone();
                    o.audio = self.selection_is_audio(&media);
                    if media.collection() {
                        o.playlist = true;
                    }
                    self.modal = Some(Modal::Form(Box::new(Form::download(media, o))));
                }
            }
            Action::Pause => {
                if self.tab == 0 {
                    self.pause_download();
                } else {
                    let _ = self.player.try_send(PlayerCommand::TogglePause);
                }
            }
            Action::Remove => {
                if self.tab == 0
                    && let Some(j) = self.snapshot.downloads.jobs.get(self.download_selected)
                {
                    let id = j.id;
                    if let Some(task) = self.running.get(&id) {
                        task.cancel.cancel();
                    }
                    self.snapshot.downloads.remove(id);
                    self.dirty = true;
                    self.download_selected = self
                        .download_selected
                        .min(self.snapshot.downloads.jobs.len().saturating_sub(1));
                }
            }
            Action::Retry => {
                if self.tab == 0
                    && let Some(j) = self.snapshot.downloads.jobs.get_mut(self.download_selected)
                    && matches!(
                        j.state,
                        JobState::Error | JobState::Paused | JobState::Cancelled
                    )
                {
                    j.invalidate(JobState::Queued);
                    j.error = None;
                    j.attempt = 0;
                    self.dirty = true;
                }
            }
            Action::Back => {
                if self.vinyl_enabled() && self.config.music_player_style.ends_with("-fullscreen") {
                    let mut config = self.config.clone();
                    config.music_player_style = self
                        .config
                        .music_player_style
                        .trim_end_matches("-fullscreen")
                        .into();
                    self.save_config(config);
                } else if self.player_status.preview {
                    let _ = self.player.try_send(PlayerCommand::ClosePreview);
                } else if self.tab > 0
                    && let Some((request, page, selected)) = self.tabs[self.tab].history.pop()
                {
                    let view = &mut self.tabs[self.tab];
                    view.cancel.cancel();
                    view.generation += 1;
                    view.page = page;
                    view.selected = selected;
                    view.request = Some(request);
                    view.loading = false;
                    view.error = None;
                    view.load = None;
                    view.empty_pages = 0;
                    view.seen_tokens.clear();
                    self.selection_changed();
                }
            }
            Action::Refresh => {
                self.card_generation = self.card_generation.wrapping_add(1);
                self.card_pending = 0;
                self.card_requests.clear();
                self.card_failures.clear();
                self.image_key = None;
                self.image_failures = 0;
                self.image_due = Instant::now();
                if self.tab > 0 && self.tabs[self.tab].load.is_some() {
                    return;
                }
                if self.tab > 0
                    && let Some(req) = self.tabs[self.tab].request.clone()
                {
                    self.browser.invalidate_page(self.tab == 2, &req);
                    self.radio.invalidate_page(&req, 0);
                    self.navigate(self.tab, req, false);
                }
            }
            Action::NextPage => {
                if self.tab > 0
                    && let Some(token) = self.tabs[self.tab].page.continuation.clone()
                {
                    let request = if self.tab == 3 {
                        self.tabs[self.tab]
                            .request
                            .clone()
                            .unwrap_or(BrowseRequest::Cliamp)
                    } else {
                        BrowseRequest::Continue(token)
                    };
                    self.navigate(self.tab, request, true);
                }
            }
            Action::Enqueue => {
                if let Some(mut m) = self.selected()
                    && !m.collection()
                {
                    if m.kind == MediaKind::Video && self.selection_is_audio(&m) {
                        m.kind = MediaKind::Song;
                    }
                    self.snapshot.playback.push(m);
                    self.status = "Added to playback queue · p to view".into();
                    self.dirty = true;
                }
            }
            Action::Favorite => {
                if let Some(m) = self.selected()
                    && m.kind == MediaKind::Station
                {
                    if let Some(i) = self.snapshot.favorites.iter().position(|v| v.url == m.url) {
                        self.snapshot.favorites.remove(i);
                        self.status = "Station removed from favorites".into();
                    } else {
                        self.snapshot.favorites.push(m);
                        self.status = "Station favorited · :favorites".into();
                    }
                    self.dirty = true;
                }
            }
            Action::Theme => {
                self.modal = Some(Modal::Themes(
                    theme::THEMES
                        .iter()
                        .position(|v| *v == self.config.theme)
                        .unwrap_or(0),
                ))
            }
            Action::Settings => {
                self.modal = Some(Modal::Form(Box::new(Form::settings(self.config.clone()))))
            }
            Action::PlaybackQueue => {
                self.modal = Some(Modal::Queue(self.snapshot.playback.current.unwrap_or(0)))
            }
            Action::NextTrack => {
                let previous = self.snapshot.playback.clone();
                if let Some(m) = self.snapshot.playback.advance(false) {
                    if !self.play_existing(m) {
                        self.snapshot.playback = previous;
                    }
                    self.dirty = true;
                } else {
                    self.snapshot.playback = previous;
                }
            }
            Action::PreviousTrack => {
                let previous = self.snapshot.playback.clone();
                if let Some(m) = self.snapshot.playback.previous() {
                    if !self.play_existing(m) {
                        self.snapshot.playback = previous;
                    }
                    self.dirty = true;
                }
            }
            Action::SeekForward => {
                let _ = self.player.try_send(PlayerCommand::Seek(5.0));
            }
            Action::SeekBack => {
                let _ = self.player.try_send(PlayerCommand::Seek(-5.0));
            }
            Action::VolumeUp => {
                let _ = self.player.try_send(PlayerCommand::Volume(5.0));
            }
            Action::VolumeDown => {
                let _ = self.player.try_send(PlayerCommand::Volume(-5.0));
            }
            Action::Mute => {
                let _ = self.player.try_send(PlayerCommand::Mute);
            }
            Action::Shuffle => {
                self.snapshot.playback.shuffle = !self.snapshot.playback.shuffle;
                self.dirty = true;
            }
            Action::Repeat => {
                self.snapshot.playback.repeat = match self.snapshot.playback.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
                self.dirty = true;
            }
            Action::SaveMedia => {
                if let Err(error) = self.command("save-media") {
                    self.status = error.to_string();
                }
            }
            Action::Stop => {
                self.pending_auto_play = None;
                let _ = self.player.try_send(PlayerCommand::Stop);
                self.snapshot.playback.current = None;
                self.dirty = true;
            }
            Action::Cliamp => {
                if self.offline {
                    self.status = "cliamp radio requires online mode".into();
                } else {
                    self.launch_cliamp = true;
                }
            }
            Action::MoveUp | Action::MoveDown => {
                if self.tab == 0 {
                    let direction = if action == Action::MoveUp { -1 } else { 1 };
                    if self
                        .snapshot
                        .downloads
                        .move_by(self.download_selected, direction)
                    {
                        self.download_selected =
                            self.download_selected.saturating_add_signed(direction);
                        self.dirty = true;
                    }
                }
            }
        }
    }
    pub fn pause_download(&mut self) {
        if let Some(job) = self.snapshot.downloads.jobs.get_mut(self.download_selected) {
            if job.state == JobState::Paused {
                job.invalidate(JobState::Queued);
            } else if job.state.active() || job.state == JobState::Queued {
                if let Some(task) = self.running.get(&job.id) {
                    task.cancel.cancel();
                }
                job.invalidate(JobState::Paused);
            }
            self.dirty = true;
        }
    }
    pub fn queue_collection(
        &mut self,
        source: String,
        title: String,
        mut options: DownloadOptions,
        selection: &str,
        items: Vec<Media>,
    ) -> Result<()> {
        options.playlist = false;
        options.playlist_items = None;
        options.validate()?;
        ensure!(
            items.len() <= 2000,
            "Collection exceeds 2,000 available entries"
        );
        let indices = history::selection_indices(selection, items.len())?;
        ensure!(
            indices
                .iter()
                .all(|index| !items[*index].collection() && http_url(&items[*index].url).is_ok()),
            "Collection contains unsupported entries; no jobs were queued"
        );
        let group = self
            .snapshot
            .downloads
            .jobs
            .iter()
            .find(|job| {
                job.group_source.as_deref() == Some(&source) && job.options.same_output(&options)
            })
            .and_then(|job| job.group_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.remember_destination(&options);
        let (mut queued, mut ready, mut shared) = (0, 0, 0);
        for index in indices {
            let media = &items[index];
            if let Some(job) = self
                .snapshot
                .downloads
                .jobs
                .iter_mut()
                .find(|job| job.media.url == media.url && job.options.same_output(&options))
            {
                if job
                    .outputs
                    .iter()
                    .any(|path| ytui_services::library::completed_file(job, path).is_some())
                {
                    ready += 1;
                } else if matches!(
                    job.state,
                    JobState::Error | JobState::Cancelled | JobState::Done
                ) {
                    job.invalidate(JobState::Queued);
                    job.error = None;
                    job.attempt = 0;
                    queued += 1;
                }
                if job.group_id.is_none() || job.group_id.as_ref() == Some(&group) {
                    job.group_id = Some(group.clone());
                    job.group_title = Some(title.clone());
                    job.group_source = Some(source.clone());
                    job.entry_index = Some(index + 1);
                } else {
                    shared += 1;
                }
            } else {
                let mut job = Job::new(media.clone(), options.clone());
                job.group_id = Some(group.clone());
                job.group_title = Some(title.clone());
                job.group_source = Some(source.clone());
                job.entry_index = Some(index + 1);
                self.snapshot.downloads.jobs.push(job);
                queued += 1;
            }
        }
        self.dirty = true;
        self.modal = Some(Modal::History(history::View {
            group: Some(group),
            ..Default::default()
        }));
        self.status = format!(
            "Collection: {queued} queued/retried · {ready} files already ready · {shared} shared in other groups"
        );
        Ok(())
    }
    pub fn enqueue_download(&mut self, mut media: Media, options: DownloadOptions) {
        if self.offline && http_url(&media.url).is_ok() {
            self.status = "Remote downloads wait until online mode".into();
            return;
        }
        if options.audio {
            media.kind = MediaKind::Song;
        }
        if self.demo {
            self.status = "Downloads are disabled for fictional demo content".into();
            return;
        }
        if options.skip_duplicates
            && self.snapshot.downloads.jobs.iter().any(|j| {
                j.media.url == media.url
                    && j.options.same_output(&options)
                    && j.state != JobState::Error
                    && j.state != JobState::Cancelled
            })
        {
            self.status = "This item is already in your download queue/history".into();
            return;
        }
        self.remember_destination(&options);
        self.snapshot.downloads.jobs.push(Job::new(media, options));
        self.dirty = true;
        self.status = "Added to Downloads · 1 opens the queue".into();
    }
    pub fn search(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        if matches!(
            self.tabs[self.tab].request,
            Some(BrowseRequest::SavedMedia { .. })
        ) {
            self.navigate(
                self.tab,
                BrowseRequest::SavedMedia {
                    query: query.into(),
                },
                false,
            );
            return;
        }
        if self.offline {
            if normalize_media_url(query).is_ok() {
                self.status = "Remote URLs require online mode".into();
                return;
            }
            let tab = if self.tab == 2 { 2 } else { 1 };
            self.tab = tab;
            self.navigate(
                tab,
                BrowseRequest::OfflineLibrary {
                    query: query.into(),
                    audio: None,
                },
                false,
            );
            return;
        }
        if let Ok(url) = normalize_media_url(query) {
            let media = Media {
                title: query.into(),
                url,
                ..Default::default()
            };
            let mut o = self.config.download.clone();
            o.audio |= self.tab == 2;
            self.modal = Some(Modal::Form(Box::new(Form::download(media, o))));
        } else if self.tab == 0 {
            self.change_tab(1);
            self.navigate(1, BrowseRequest::Search(query.into()), false);
        } else {
            self.navigate(
                self.tab,
                if self.tab == 3 {
                    BrowseRequest::RadioSearch(query.into())
                } else {
                    BrowseRequest::Search(query.into())
                },
                false,
            );
        }
    }
    fn selection_is_audio(&self, media: &Media) -> bool {
        matches!(media.kind, MediaKind::Song | MediaKind::Station)
            || (self.tab == 2
                && !matches!(
                    self.tabs[self.tab].request,
                    Some(BrowseRequest::SavedMedia { .. })
                ))
    }
    pub fn play(&mut self, mut media: Media) {
        if media.kind == MediaKind::Video && self.selection_is_audio(&media) {
            media.kind = MediaKind::Song;
        }
        if self.offline && http_url(&media.url).is_ok() {
            self.status = "Offline mode can play local media only".into();
            return;
        }
        if self.demo {
            self.status = "Playback is disabled for fictional demo content".into();
            return;
        }
        if !self.play_existing(media.clone()) {
            return;
        }
        // Opening a song starts the remaining songs from the current collection.
        // Live stations keep their continuous stream and do not enqueue catalog tracks.
        if media.kind == MediaKind::Song && self.tab > 0 {
            let items = &self.tabs[self.tab].page.items;
            if let Some(index) = items.iter().position(|m| m.url == media.url) {
                let queue = &mut self.snapshot.playback;
                queue
                    .items
                    .truncate(queue.current.map(|i| i + 1).unwrap_or(queue.items.len()));
                queue.play(media.clone());
                for next in items
                    .iter()
                    .skip(index + 1)
                    .filter(|m| m.kind == MediaKind::Song)
                {
                    queue.push(next.clone());
                }
            } else {
                self.snapshot.playback.play(media.clone());
            }
        } else {
            self.snapshot.playback.play(media.clone());
        }
        self.dirty = true;
    }
    pub fn resume_positions(&self) -> Vec<&ytui_core::playback_history::Position> {
        self.snapshot
            .positions
            .resumable()
            .into_iter()
            .filter(|p| !self.offline || http_url(&p.media.url).is_err())
            .collect()
    }
    pub fn resume_position(&mut self, position: ytui_core::playback_history::Position) {
        if self.player_session_pending {
            self.status = "Updating player account · retry resume shortly".into();
            return;
        }
        if self.offline && http_url(&position.media.url).is_ok() {
            self.status = "Resume this online video after restarting without --offline".into();
            return;
        }
        if self.demo {
            self.status = "Playback is disabled in demo mode".into();
            return;
        }
        let audio = position.media.kind == MediaKind::Song;
        let command = PlayerCommand::PlayAt {
            media: position.media.clone(),
            audio,
            seconds: position.seconds,
        };
        if self.player.try_send(command).is_err() {
            self.status = "Player is busy; try resume again".into();
            return;
        }
        self.snapshot.playback.play(position.media);
        self.dirty = true;
        self.status = format!(
            "Resuming at {}:{:02}",
            position.seconds as u64 / 60,
            position.seconds as u64 % 60
        );
    }
    pub fn player_update(&mut self, status: PlayerStatus) {
        if self.vinyl_enabled()
            && self.player_status.media.as_ref().map(|m| &m.thumbnail)
                != status.media.as_ref().map(|m| &m.thumbnail)
        {
            self.selection_changed();
        }
        if !self.demo
            && self.config.playback_history
            && status.active
            && !status.preview
            && !status.buffering
            && let Some(media) = &status.media
            && !self.player_session_pending
            && self
                .snapshot
                .playback
                .current
                .and_then(|index| self.snapshot.playback.items.get(index))
                .is_some_and(|current| current.url == media.url)
        {
            self.dirty |=
                self.snapshot
                    .positions
                    .record(media, status.position, status.duration, false);
        }
        self.player_status = status;
        self.redraw = true;
    }
    pub fn playback_ended(&mut self, media: &Media) {
        if self.enforce_sleep_timer(Instant::now()) {
            return;
        }
        if !self
            .snapshot
            .playback
            .current
            .and_then(|index| self.snapshot.playback.items.get(index))
            .is_some_and(|current| current.url == media.url)
        {
            return;
        }
        if self.config.playback_history && !self.demo {
            self.snapshot.positions.record(
                media,
                self.player_status
                    .duration
                    .unwrap_or(self.player_status.position),
                self.player_status.duration,
                true,
            );
        }
        if let Some(media) = self.snapshot.playback.advance(true)
            && !self.play_existing(media.clone())
        {
            self.pending_auto_play = Some(media);
        }
        self.dirty = true;
    }
    fn sync_player_session(&mut self) {
        if self.player_session_pending
            && self
                .player
                .try_send(PlayerCommand::Session(self.session.clone()))
                .is_ok()
        {
            self.player_session_pending = false;
        }
    }
    pub fn play_existing(&mut self, mut media: Media) -> bool {
        if self.enforce_sleep_timer(Instant::now()) {
            return false;
        }
        if self.player_session_pending {
            self.status = "Updating player account · retry playback shortly".into();
            return false;
        }
        if self.offline && http_url(&media.url).is_ok() {
            self.status = "Queued online media waits for online mode".into();
            return false;
        }
        if http_url(&media.url).is_err()
            && let Ok(path) = std::fs::canonicalize(&media.url)
        {
            media.url = path.to_string_lossy().into();
        }
        let audio = matches!(media.kind, MediaKind::Song | MediaKind::Station);
        if self
            .player
            .try_send(PlayerCommand::Play { media, audio })
            .is_err()
        {
            self.status = "Player is busy; retry playback".into();
            return false;
        }
        true
    }
    pub fn sleep_label(&self) -> Option<String> {
        if self.sleep_timer.stopping() {
            return Some(self.tr("Sleep timer · stopping playback").into());
        }
        self.sleep_timer.remaining(Instant::now()).map(|seconds| {
            format!(
                "{} {:02}:{:02}:{:02}",
                self.tr("Sleep"),
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            )
        })
    }
    pub fn enforce_sleep_timer(&mut self, now: Instant) -> bool {
        if !self.sleep_timer.due(now) {
            return false;
        }
        self.pending_auto_play = None;
        self.snapshot.playback.current = None;
        self.dirty = true;
        self.redraw = true;
        match self.player.try_send(PlayerCommand::Stop) {
            Ok(()) => {
                self.sleep_timer.cancel();
                self.status = "Sleep timer finished · playback stopped".into();
            }
            Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.sleep_timer.cancel();
                self.status = "Sleep timer could not reach player · open Diagnostics".into();
            }
        }
        true
    }
    pub async fn tick(&mut self) {
        let spinning = self.vinyl_enabled()
            && self.player_status.active
            && !self.player_status.paused
            && !self.player_status.buffering
            && !self.config.reduced_motion;
        if spinning || self.visual_levels.is_none() {
            self.visual_levels = self.player_status.audio_levels;
        }
        self.vinyl_motion.update(Instant::now(), spinning);
        self.redraw |= spinning;
        self.enforce_sleep_timer(Instant::now());
        self.redraw |= self.sleep_timer.needs_redraw(Instant::now());
        self.sync_player_session();
        if let Some(media) = self.pending_auto_play.take()
            && self
                .snapshot
                .playback
                .current
                .and_then(|index| self.snapshot.playback.items.get(index))
                .is_some_and(|current| current.url == media.url)
            && (self.offline && http_url(&media.url).is_ok() || !self.play_existing(media.clone()))
        {
            self.pending_auto_play = Some(media);
        }
        self.recover_feed();
        self.prefetch_page();
        let finished = self
            .running
            .iter()
            .filter(|(_, task)| task.task.is_finished())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in finished {
            if let Some(task) = self.running.remove(&id)
                && task.task.await.is_err()
                && let Some(job) = self
                    .snapshot
                    .downloads
                    .jobs
                    .iter_mut()
                    .find(|j| j.id == id && j.state.active())
            {
                job.state = JobState::Error;
                job.error = Some("Download worker exited unexpectedly".into());
                self.dirty = true;
                self.redraw = true;
            }
        }
        if !self.demo {
            let policy = self.config.schedule.policy(
                ytui_services::desktop::local_minute(),
                self.config.download.global_bandwidth_kib,
                self.config.concurrency,
            );
            if self.policy.is_some_and(|old| old != policy) || !policy.allowed {
                for job in &mut self.snapshot.downloads.jobs {
                    if job.state.active() {
                        if let Some(task) = self.running.get(&job.id) {
                            task.cancel.cancel();
                        }
                        job.invalidate(JobState::Queued);
                        self.dirty = true;
                    }
                }
                if self.policy != Some(policy) {
                    self.status = if policy.allowed {
                        "Download schedule/rate changed · resuming eligible jobs"
                    } else {
                        "Outside download window · queue waits; manual pauses are preserved"
                    }
                    .into();
                    self.redraw = true;
                }
            }
            self.policy = Some(policy);
            let now = schedule::unix_now();
            for job in &mut self.snapshot.downloads.jobs {
                if self.running.len() >= self.config.concurrency {
                    break;
                }
                if policy.allowed
                    && job.ready(now)
                    && (!self.offline || http_url(&job.media.url).is_err())
                    && !self.running.contains_key(&job.id)
                {
                    job.state = JobState::Resolving;
                    let cancel = CancellationToken::new();
                    let mut transfer = job.clone();
                    transfer.options.global_bandwidth_kib = policy.global_bandwidth_kib;
                    let task = tokio::spawn(download::execute(
                        transfer,
                        self.config.clone(),
                        self.paths.clone(),
                        self.session.clone(),
                        cancel.clone(),
                        self.job_tx.clone(),
                    ));
                    self.running.insert(job.id, Running { cancel, task });
                    self.dirty = true;
                    self.redraw = true;
                }
            }
        }
        if !self.demo && !self.offline {
            let owner = self.watch_owner();
            let now = schedule::unix_now();
            for watch in &mut self.snapshot.watches {
                if watch.enabled && watch.owner != owner {
                    watch.enabled = false;
                    watch.generation += 1;
                    watch.error =
                        Some("Account changed; resume explicitly to reset the baseline".into());
                    self.dirty = true;
                }
                if watch.enabled
                    && self.watch_pending.len() < 2
                    && watch.next_check <= now
                    && !self.watch_pending.contains_key(&watch.id)
                {
                    let cancel = self.desktop_cancel.child_token();
                    let id = watch.id;
                    let generation = watch.generation;
                    self.watch_pending.insert(id, (generation, cancel.clone()));
                    let watch = watch.clone();
                    let browser = self.browser.clone();
                    self.tasks.spawn(async move {
                        UiEvent::Watch {
                            id,
                            generation,
                            result: ytui_services::watch::collect(browser, watch, cancel).await,
                        }
                    });
                }
            }
        }
        if !self.demo
            && self.config.clipboard_watcher
            && !self.clipboard_pending
            && Instant::now() >= self.clipboard_due
        {
            self.clipboard_pending = true;
            let generation = self.clipboard_generation;
            let cancel = self.clipboard_cancel.child_token();
            self.tasks.spawn(async move {
                UiEvent::Clipboard {
                    generation,
                    result: ytui_services::desktop::read_clipboard(cancel).await,
                }
            });
        }
        if self.theme_rx.has_changed().unwrap_or(false) {
            self.palette = self.theme_rx.borrow_and_update().clone();
            if self.demo {
                self.selection_changed();
            }
            self.redraw = true;
        }
        self.refresh_image();
    }
    fn watch_owner(&self) -> String {
        self.session
            .as_ref()
            .map(|s| format!("{}:{}", s.account, self.config.account_index))
            .unwrap_or_else(|| "anonymous".into())
    }
    fn toggle_watch(&mut self, id: uuid::Uuid, enabled: bool) {
        let owner = self.watch_owner();
        if let Some((_, token)) = self.watch_pending.remove(&id) {
            token.cancel();
        }
        if let Some(watch) = self.snapshot.watches.iter_mut().find(|w| w.id == id) {
            watch.generation += 1;
            watch.enabled = enabled;
            watch.next_check = schedule::unix_now();
            if enabled && watch.owner != owner {
                watch.owner = owner;
                watch.seen = None;
            }
            self.dirty = true;
        }
    }
    pub fn download_groups(&self) -> Vec<(String, String, usize)> {
        let mut groups: Vec<(String, String, usize)> = Vec::new();
        for job in &self.snapshot.downloads.jobs {
            if let Some(id) = &job.group_id {
                if let Some(group) = groups.iter_mut().find(|g| &g.0 == id) {
                    group.2 += 1;
                } else {
                    groups.push((
                        id.clone(),
                        job.group_title
                            .clone()
                            .unwrap_or_else(|| job.media.title.clone()),
                        1,
                    ));
                }
            }
        }
        groups
    }
    pub fn job_event(&mut self, event: JobEvent) {
        let selected_history_id = if let Some(Modal::History(view)) = &self.modal {
            view.rows(&self.snapshot.downloads.jobs)
                .get(view.selected)
                .map(|(_, j)| j.id)
        } else {
            None
        };
        let terminal = matches!(event.update, JobUpdate::Finished | JobUpdate::Failed { .. });
        let id = event.id;
        if !self.snapshot.downloads.apply(event) {
            return;
        }
        if let Some(id) = selected_history_id
            && let Some(Modal::History(view)) = &mut self.modal
            && let Some(index) = view
                .rows(&self.snapshot.downloads.jobs)
                .iter()
                .position(|(_, j)| j.id == id)
        {
            view.selected = index;
        }
        self.dirty = true;
        self.redraw = true;
        if terminal
            && !self.demo
            && self.config.notifications
            && self.notification_pending < 16
            && let Some(job) = self.snapshot.downloads.jobs.iter().find(|j| j.id == id)
        {
            let failed = job.state == JobState::Error;
            let title = if failed {
                "Download failed"
            } else {
                "Download complete"
            };
            let text = job.media.title.clone();
            let sound = self.config.notification_sound;
            self.notification_pending += 1;
            self.tasks.spawn(async move {
                UiEvent::Notification(
                    ytui_services::desktop::notify(title, &text, failed, sound).await,
                )
            });
        }
    }
    pub async fn shutdown(&mut self) {
        self.collection_cancel.cancel();
        self.operations.cancel();
        self.desktop_cancel.cancel();
        self.clipboard_cancel.cancel();
        self.theme_cancel.cancel();
        self.image_cancel.cancel();
        for tab in &self.tabs {
            tab.cancel.cancel();
        }
        for task in self.running.values() {
            task.cancel.cancel();
        }
        for (_, task) in self.running.drain() {
            let _ = task.task.await;
        }
        // Give cancelled supervised desktop processes time to reap before aborting network tasks.
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while self.tasks.join_next().await.is_some() {}
        })
        .await;
        self.tasks.shutdown().await;
        for job in &mut self.snapshot.downloads.jobs {
            if job.state.active() {
                job.invalidate(JobState::Queued);
            }
        }
    }
    fn import_auth(&mut self, browser: String, profile: String, file: String) {
        if self.pending_operation.is_some() || self.disconnect_pending {
            self.status =
                "Account/settings operation in progress · wait or cancel it in Actions".into();
            return;
        }
        if self.offline {
            self.status = "Account validation requires online mode".into();
            return;
        }
        if self.demo {
            self.status = "Account access is disabled in demo mode".into();
            return;
        }
        self.status = "Reading selected profile and validating session…".into();
        let store = SessionStore::new(&self.paths, &self.config.auth_store);
        let config = self.config.clone();
        let operation = self.operations.account();
        self.pending_operation = Some((operation.clone(), "Validating selected account…".into()));
        let operations = self.operations.clone();
        self.tasks.spawn(async move {
            let result = async {
                let candidate = operation
                    .prepare(async {
                        tokio::task::spawn_blocking(move || {
                            if file.is_empty() {
                                auth::import_browser(
                                    &browser,
                                    if profile.is_empty() {
                                        None
                                    } else {
                                        Some(std::path::Path::new(&profile))
                                    },
                                )
                            } else {
                                auth::import_netscape(std::path::Path::new(&file))
                            }
                        })
                        .await?
                    })
                    .await?;
                let api = Browser::new(
                    ytui_services::http_client(config.proxy.as_deref())?,
                    Some(Arc::new(candidate.clone())),
                    config.account_index,
                );
                let mut candidate = candidate;
                candidate.account = operation.prepare(api.validate()).await?;
                let saved = candidate.clone();
                operations
                    .commit(operation.clone(), move || store.save(&saved))
                    .await?;
                Ok(candidate)
            }
            .await;
            UiEvent::Auth { operation, result }
        });
    }
    pub fn open_action(&mut self, index: usize) {
        let Some(entry) = crate::actions::ENTRIES.get(index) else {
            return;
        };
        self.modal = None;
        if !entry.fields.is_empty() {
            let mut form = Form::action(entry);
            if matches!(entry.command, "lyrics-find" | "lyrics-local") {
                let media = self.player_status.media.clone().or_else(|| self.selected());
                if let FormKind::Action { media_url, .. } = &mut form.kind {
                    *media_url = media.as_ref().map(|media| media.url.clone());
                }
                if entry.command == "lyrics-find"
                    && let Some(media) = media
                {
                    form.fields[0].value = Editor::new(media.title);
                    form.fields[1].value = Editor::new(media.creator);
                }
            }
            self.modal = Some(Modal::Form(Box::new(form)));
        } else if let Err(e) = self.command(entry.command) {
            self.status = e.to_string();
        }
    }
    pub fn command(&mut self, input: &str) -> Result<()> {
        let parts = shell_words::split(input)?;
        let Some(cmd) = parts.first().map(String::as_str) else {
            return Ok(());
        };
        let slash = cmd.starts_with('/');
        let cmd = cmd.trim_start_matches('/');
        let cmd = if slash && cmd == "history" {
            "downloads"
        } else {
            cmd
        };
        let value = parts.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        let browse_tab = if self.tab == 0 { 1 } else { self.tab };
        match cmd {
            "lyrics-find" | "lyrics-local" => {
                let remote = cmd == "lyrics-find";
                ensure!(
                    !remote || (!self.offline && !self.demo),
                    "Online lyrics are unavailable in offline/demo mode; choose a local lyrics file"
                );
                ensure!(
                    !remote || parts.len() == 3,
                    "Review the song title and artist in Actions → Find lyrics (LRCLIB)"
                );
                ensure!(
                    remote || !value.trim().is_empty(),
                    "Choose an LRC or text file"
                );
                let media_url = self
                    .player_status
                    .media
                    .as_ref()
                    .map(|m| m.url.clone())
                    .or_else(|| self.selected().map(|m| m.url));
                let view = crate::lyrics::View::new(
                    media_url,
                    if remote { "LRCLIB" } else { "Local file" },
                );
                let id = view.id;
                let cancel = view.cancel.clone();
                let proxy = self.config.proxy.clone();
                let title = parts.get(1).cloned().unwrap_or_default();
                let artist = parts.get(2).cloned().unwrap_or_default();
                self.tasks.spawn(async move {
                    let result = tokio::select! {
                        _ = cancel.cancelled() => Err(anyhow::anyhow!("Lyrics request cancelled")),
                        result = async {
                            if remote {
                                ytui_services::lyrics::lookup(proxy.as_deref(), &title, &artist).await
                            } else {
                                ytui_services::lyrics::local(std::path::Path::new(&value)).await.map(|track| vec![track])
                            }
                        } => result,
                    };
                    UiEvent::Lyrics { id, result }
                });
                self.modal = Some(Modal::Lyrics(Box::new(view)));
            }
            "music-player-style" => self.action(Action::MusicPlayerStyle),
            "hints" => self.action(Action::ToggleHints),
            "categories" => self.action(Action::Categories),
            "download-options" => self.action(Action::Download),
            "fallback" => self.action(Action::Fallback),
            "launch-cliamp" => self.action(Action::Cliamp),
            "playback-settings" => {
                let mut form = Form::settings(self.config.clone());
                form.select_section(8);
                self.modal = Some(Modal::Form(Box::new(form)));
            }
            "backend" => {
                let mut config = self.config.clone();
                config.download_backend = value;
                config.validate()?;
                self.save_config(config);
            }
            "playback-backend" => {
                let mut config = self.config.clone();
                config.playback_backend = value;
                config.validate()?;
                self.save_config(config);
            }
            "native-runtime" => {
                let path = ytui_services::paths::executable(&value).context("Runtime was not found. Install Node.js 22+ or Deno 2.9.7, or clear the runtime field in Settings → Tools for automatic detection.")?;
                let mut config = self.config.clone();
                config.native_js_runtime = Some(path);
                self.save_config(config);
            }
            "sponsor-status" => {
                self.modal = Some(Modal::Text { title: "SponsorBlock & ad handling".into(), offset: 0, lines: vec![
                    format!("Configured mode: {} · temporary skipping: {}", self.config.playback_sponsorblock, self.player_status.sponsor_enabled),
                    format!("Categories: {} · fetched marks: {}", self.config.playback_sponsor_categories.join(", "), self.player_status.segments.len()),
                    self.player_status.notice.clone().unwrap_or_else(|| "No current API result".into()),
                    "Skip jumps past matching sponsor segments; Mark displays them without seeking; Off makes no startup API request.".into(),
                    "B toggles skipping for this playback and fetches marks if needed. U replays the last skipped part.".into(),
                    "An API failure never stops the video. Configure categories/mode in Settings → Playback.".into(),
                    "mpv plays extracted media directly, without the website's ad UI. SponsorBlock uses public video IDs to mark embedded sponsors.".into(),
                    "This is not universal browser adblocking or guaranteed removal of server-inserted ads (SSAI).".into(),
                ] });
            }
            "tracks" => {
                let media = self.selected().context("Select a station first")?;
                let url = media
                    .tracks_url
                    .context("This station exposes a live stream only")?;
                self.change_tab(3);
                self.navigate(3, BrowseRequest::CliampTracks(url), false);
            }
            "player-formats" => {
                self.modal = Some(Modal::PlayerSettings {
                    section: 0,
                    selected: 0,
                })
            }
            "add-station" => {
                let name = parts.get(1).context("Provide a station name")?;
                let url = parts.get(2).context("Provide a stream URL")?;
                ytui_core::http_url(url)?;
                let mut config = self.config.clone();
                config.custom_stations.push(Media {
                    id: uuid::Uuid::new_v4().to_string(),
                    title: name.clone(),
                    url: url.clone(),
                    kind: MediaKind::Station,
                    live: true,
                    ..Default::default()
                });
                self.save_config(config);
            }
            "player-quality" => {
                ensure!(
                    value == "best"
                        || value
                            .parse::<u32>()
                            .is_ok_and(|v| (144..=8640).contains(&v)),
                    "Quality must be best or a resolution height"
                );
                let _ = self.player.try_send(PlayerCommand::Quality(value));
            }
            "player-seek" => {
                let seconds = value.parse::<f64>()?;
                ensure!(
                    seconds.is_finite() && seconds >= 0.0,
                    "Position must be a finite number of seconds"
                );
                let _ = self.player.try_send(PlayerCommand::SeekTo(seconds));
            }
            cmd if cmd.starts_with("player-") => {
                let action = match cmd {
                    "player-pause" => Action::Pause,
                    "player-queue" => Action::PlaybackQueue,
                    "player-next" => Action::NextTrack,
                    "player-previous" => Action::PreviousTrack,
                    "player-volume-up" => Action::VolumeUp,
                    "player-volume-down" => Action::VolumeDown,
                    "player-mute" => Action::Mute,
                    "player-speed-up" => Action::SpeedUp,
                    "player-speed-down" => Action::SpeedDown,
                    "player-fullscreen" => Action::Fullscreen,
                    "player-audio" => Action::CycleAudio,
                    "player-subtitle" => Action::CycleSubtitles,
                    "player-subtitle-toggle" => Action::ToggleSubtitles,
                    "player-chapter-next" => Action::NextChapter,
                    "player-chapter-previous" => Action::PreviousChapter,
                    "player-sponsor-toggle" => Action::SponsorToggle,
                    "player-sponsor-undo" => Action::SponsorUndo,
                    "player-shuffle" => Action::Shuffle,
                    "player-repeat" => Action::Repeat,
                    "player-stop" => Action::Stop,
                    _ => anyhow::bail!("Unknown player control"),
                };
                if action == Action::Pause {
                    let _ = self.player.try_send(PlayerCommand::TogglePause);
                } else {
                    self.action(action);
                }
            }
            "save-media" => {
                ensure!(!self.demo, "Saving fictional demo content is disabled");
                let media = self.selected().context("Select a video or song first")?;
                let saved = self.snapshot.saved_media.toggle(media)?;
                self.dirty = true;
                self.status = if saved {
                    "Saved locally"
                } else {
                    "Removed from saved shelf"
                }
                .into();
                if let Some(BrowseRequest::SavedMedia { query }) =
                    self.tabs[self.tab].request.clone()
                {
                    self.navigate(self.tab, BrowseRequest::SavedMedia { query }, false);
                }
            }
            "saved" => {
                ensure!(value.len() <= 1024, "Saved search is limited to 1024 bytes");
                if self.tab == 0 || self.tab == 3 {
                    self.tab = 1;
                }
                self.navigate(self.tab, BrowseRequest::SavedMedia { query: value }, false);
            }
            "sleep" if value == "off" => {
                self.sleep_timer.cancel();
                self.status = "Sleep timer cancelled".into();
            }
            "sleep" if value.is_empty() => {
                self.modal = Some(Modal::Text { title: self.tr("Sleep timer").into(), offset: 0,
                    lines: vec![self.sleep_label().unwrap_or_else(|| self.tr("Sleep timer is off").into()),
                        self.tr("Set or cancel the timer in Actions → Player. Playback stops without quitting ytui; the queue stays saved. The timer lasts for this session only.").into()] });
            }
            "sleep" => {
                let seconds = schedule::delay_seconds(&value)?;
                self.sleep_timer
                    .arm(Duration::from_secs(seconds), Instant::now())?;
                self.status = format!(
                    "Sleep timer set · {}",
                    self.sleep_label().unwrap_or_default()
                );
            }
            "quit" | "q" => self.quit = true,
            "feeds" => self.modal = Some(Modal::Feeds(0)),
            "watch" | "watches" if value.is_empty() || value == "list" => {
                self.modal = Some(Modal::Watches(0))
            }
            "watch" if parts.get(1).is_some_and(|p| p == "add") => {
                ensure!(!self.demo, "Collection watches are disabled in demo mode");
                ensure!(
                    self.tab == 1 || self.tab == 2,
                    "Open a YouTube/Music collection first"
                );
                let interval = schedule::delay_seconds(
                    parts
                        .get(2)
                        .context("Use :watch add 30m while viewing a collection")?,
                )?;
                let request = self.tabs[self.tab]
                    .request
                    .clone()
                    .context("Open a collection first")?;
                let watch = ytui_core::watch::CollectionWatch {
                    id: uuid::Uuid::new_v4(),
                    title: self.tabs[self.tab].page.title.clone(),
                    request,
                    music: self.tab == 2,
                    options: self.config.download.clone(),
                    interval_seconds: interval,
                    next_check: 0,
                    enabled: true,
                    generation: 0,
                    owner: self.watch_owner(),
                    seen: None,
                    failures: 0,
                    error: None,
                };
                watch.validate()?;
                ensure!(
                    !self
                        .snapshot
                        .watches
                        .iter()
                        .any(|w| w.request == watch.request
                            && w.music == watch.music
                            && w.owner == watch.owner),
                    "This collection already has a watch"
                );
                let mut watch = watch;
                watch.options.audio = self.tab == 2;
                self.snapshot.watches.push(watch);
                self.dirty = true;
                self.status =
                    "Watch added · establishing a baseline; existing entries will not download"
                        .into();
            }
            "watch" => {
                let action = parts.get(1).context("Use :watch add/pause/resume/remove")?;
                let id = uuid::Uuid::parse_str(
                    parts.get(2).context("Provide a watch UUID from :watches")?,
                )?;
                ensure!(
                    self.snapshot.watches.iter().any(|w| w.id == id),
                    "Watch not found"
                );
                ensure!(
                    ["pause", "resume", "remove"].contains(&action.as_str()),
                    "Use :watch pause/resume/remove UUID"
                );
                self.toggle_watch(id, action == "resume");
                if action == "remove" {
                    self.snapshot.watches.retain(|w| w.id != id);
                }
                self.dirty = true;
            }
            "offline" => {
                let mut words = value.split_whitespace();
                let audio = match words.next() {
                    Some("audio") => Some(true),
                    Some("video") => Some(false),
                    _ => None,
                };
                let query = if audio.is_some() {
                    words.collect::<Vec<_>>().join(" ")
                } else {
                    value
                };
                let tab = if audio == Some(true) || self.tab == 2 {
                    2
                } else {
                    1
                };
                self.tab = tab;
                self.navigate(tab, BrowseRequest::OfflineLibrary { query, audio }, false);
            }
            "resume" if value == "clear" => {
                self.snapshot.positions.entries.clear();
                self.dirty = true;
                self.status = "Local playback positions cleared".into();
            }
            "resume" if value == "selected" => {
                let media = self.selected_for_resume().context("Select an item first")?;
                let position = self
                    .snapshot
                    .positions
                    .for_media(&media)
                    .cloned()
                    .context("No saved position for this media")?;
                self.resume_position(position);
            }
            "resume" => self.modal = Some(Modal::Resume(0)),
            "thumbnail-cache" | "thumbnail-cache-clear" => {
                let images = self.images.clone();
                let clear = cmd == "thumbnail-cache-clear";
                self.status = if clear {
                    "Clearing thumbnail cache…"
                } else {
                    "Inspecting thumbnail cache…"
                }
                .into();
                let id = if clear {
                    uuid::Uuid::nil()
                } else {
                    self.pending_text("Thumbnail cache")
                };
                self.tasks.spawn(async move {
                    let result = images.inspect(clear).await;
                    if clear { UiEvent::CacheCleared(result) } else {
                        UiEvent::Text { id, result: result.map(|stats| vec![
                            format!("Disk: {} images · {:.1} MiB / 128 MiB", stats.files, stats.disk_bytes as f64 / 1048576.0),
                            format!("Decoded memory: {} images · {:.1} MiB / 32 MiB", stats.memory_images, stats.memory_bytes as f64 / 1048576.0),
                            "Encoded terminal cards are separately limited to visible images.".into(),
                            "Actions → Clear thumbnail cache removes cached images only.".into(),
                            "Temporary network failures retry automatically with a bounded backoff.".into(),
                        ]) }
                    }
                });
            }
            "capabilities" | "support-export" => {
                let config = self.config.clone();
                let active_backend = self.player_status.backend.clone();
                let error = self.player_status.error.clone().unwrap_or_default();
                let cancel = self.desktop_cancel.child_token();
                let output = (cmd == "support-export").then(|| PathBuf::from(&value));
                ensure!(
                    output
                        .as_ref()
                        .is_none_or(|path| !path.as_os_str().is_empty()),
                    "Choose a new JSON file"
                );
                self.status = "Checking installed tool capabilities…".into();
                let id = self.pending_text("Setup & capabilities / support report");
                self.tasks.spawn(async move {
                    let report = ytui_services::diagnostics::Support::collect(
                        &config,
                        env!("CARGO_PKG_VERSION"),
                        env!("YTUI_BUILD_DESCRIPTION"),
                        &error,
                        cancel,
                    )
                    .await
                    .with_active_backend(&active_backend);
                    let result = async {
                        if let Some(path) = output {
                            let data = serde_json::to_vec_pretty(&report)?;
                            tokio::task::spawn_blocking(move || {
                                ytui_services::paths::atomic_write_new(&path, &data)
                            })
                            .await??;
                        }
                        Ok(report.lines())
                    }
                    .await;
                    UiEvent::Text { id, result }
                });
            }
            "log" | "diagnostics" => {
                let mut lines = vec![
                    format!(
                        "ytui {} · build {} · downloads {} · playback {}",
                        env!("CARGO_PKG_VERSION"),
                        env!("YTUI_BUILD_DESCRIPTION"),
                        self.config.download_backend,
                        self.config.playback_backend
                    ),
                    format!("Theme: {} · {}", self.palette.name, self.palette.source),
                    format!("Active playback backend: {}", self.player_status.backend),
                    "Browser credentials and signed URLs are never included.".into(),
                    String::new(),
                ];
                lines.extend(
                    self.snapshot
                        .downloads
                        .jobs
                        .iter()
                        .rev()
                        .filter_map(|j| {
                            j.error.as_ref().map(|e| {
                                format!("{} · {}", j.id, ytui_services::process::diagnostic(e))
                            })
                        })
                        .take(30),
                );
                if let Some(error) = &self.player_status.error {
                    lines.push(format!(
                        "Inferred failure phase: {}",
                        ytui_services::diagnostics::phase(error)
                    ));
                    lines.push(ytui_services::process::diagnostic(error));
                }
                self.modal = Some(Modal::Text {
                    title: "Diagnostics".into(),
                    lines,
                    offset: 0,
                });
            }
            "update" => {
                self.modal = Some(Modal::Text {
                    title: "Update ytui".into(),
                    lines: vec![
                        "Native extraction is compiled into the Rust application.".into(),
                        "Update the checkout, review changes, then run ./install-rust.sh.".into(),
                        "Pinned native dependencies are updated with reviewed source changes."
                            .into(),
                        "The optional Python yt-dlp fallback is managed by your package manager."
                            .into(),
                        "No executable or dependency was downloaded/replaced by this command."
                            .into(),
                    ],
                    offset: 0,
                });
            }
            "help" | "?" => self.modal = Some(Modal::Help(0)),
            "keys" => self.modal = Some(Modal::Form(Box::new(Form::keys(self.config.clone())))),
            "downloads" | "stats" => {
                let mut words = value.split_whitespace();
                let filter = words.next().and_then(history::Filter::parse);
                let query = if filter.is_some() {
                    words.collect::<Vec<_>>().join(" ")
                } else {
                    value
                };
                self.modal = Some(Modal::History(history::View {
                    filter: filter.unwrap_or_default(),
                    query,
                    ..Default::default()
                }));
            }
            "queue" => self.change_tab(0),
            "groups" => self.modal = Some(Modal::Groups(0)),
            "download-files" => {
                let job = self
                    .snapshot
                    .downloads
                    .jobs
                    .get(self.download_selected)
                    .context("Select a download first")?;
                self.modal = Some(Modal::Outputs {
                    job: job.id,
                    selected: 0,
                });
            }
            "download-recent" => self.modal = Some(Modal::Destinations(0)),
            "download-collection" => {
                ensure!(
                    !self.demo && !self.offline,
                    "Collection expansion requires online mode"
                );
                ensure!(
                    !self.collection_pending,
                    "A collection is already loading; use Cancel collection loading in Actions"
                );
                let tab = if self.tab == 2 { 2 } else { 1 };
                let selected = self.selected();
                let (request, title) =
                    if let Some(media) = selected.filter(|media| media.collection()) {
                        (
                            BrowseRequest::Collection(
                                media
                                    .browse_id
                                    .context("This collection has no browse ID")?,
                            ),
                            media.title,
                        )
                    } else {
                        (
                            self.tabs[tab]
                                .request
                                .clone()
                                .filter(|request| matches!(request, BrowseRequest::Collection(_)))
                                .context("Open a playlist/album/channel first")?,
                            self.tabs[tab].page.title.clone(),
                        )
                    };
                let source = serde_json::to_string(&request)?;
                let options = self.config.download.clone();
                let mut entry_options = options.clone();
                entry_options.audio = tab == 2;
                entry_options.playlist = false;
                entry_options.playlist_items = None;
                let watch = watch::CollectionWatch {
                    id: uuid::Uuid::new_v4(),
                    title: title.clone(),
                    request,
                    music: tab == 2,
                    options,
                    interval_seconds: 60,
                    next_check: 0,
                    enabled: false,
                    generation: 0,
                    owner: self.watch_owner(),
                    seen: None,
                    failures: 0,
                    error: None,
                };
                self.collection_cancel = CancellationToken::new();
                self.collection_generation = self.collection_generation.wrapping_add(1);
                self.collection_pending = true;
                let cancel = self.collection_cancel.clone();
                let generation = self.collection_generation;
                let browser = self.browser.clone();
                self.status =
                    "Loading every collection page · no entries queued until expansion succeeds"
                        .into();
                self.tasks.spawn(async move {
                    UiEvent::Collection {
                        generation,
                        source,
                        title,
                        options: Box::new(entry_options),
                        selection: value,
                        result: ytui_services::watch::collect(browser, watch, cancel).await,
                    }
                });
            }
            "cancel-collection" => {
                self.collection_cancel.cancel();
                self.collection_generation = self.collection_generation.wrapping_add(1);
                self.collection_pending = false;
                self.status = "Collection loading cancelled · existing downloads retained".into();
            }
            "download-page" => {
                ensure!(
                    self.tab == 1 || self.tab == 2,
                    "Open a YouTube/Music collection first"
                );
                ensure!(!self.demo, "Downloads are disabled in demo mode");
                let page = &self.tabs[self.tab].page;
                let indices = history::selection_indices(&value, page.items.len())?;
                let media = indices
                    .into_iter()
                    .map(|i| page.items[i].clone())
                    .collect::<Vec<_>>();
                ensure!(
                    media
                        .iter()
                        .all(|m| !m.collection() && http_url(&m.url).is_ok()),
                    "Select individual videos/songs only; open collections first"
                );
                let group = uuid::Uuid::new_v4().to_string();
                let mut added = 0;
                for media in media {
                    let before = self.snapshot.downloads.jobs.len();
                    let mut options = self.config.download.clone();
                    options.audio = self.selection_is_audio(&media);
                    self.enqueue_download(media, options);
                    if self.snapshot.downloads.jobs.len() > before {
                        self.snapshot.downloads.jobs.last_mut().unwrap().group_id =
                            Some(group.clone());
                        added += 1;
                    }
                }
                self.status =
                    format!("Added {added} items from the loaded page · F2 → Download groups");
            }
            "audio" | "video" | "audio-playlist" | "video-playlist" => {
                let mut config = self.config.clone();
                config.download.audio = cmd.starts_with("audio");
                config.download.playlist = cmd.ends_with("playlist");
                self.save_config(config);
            }
            "bandwidth" | "limit" | "proxy" | "subs" | "subtitles" | "metadata" | "hooks" => {
                self.modal = Some(Modal::Form(Box::new(Form::settings(self.config.clone()))));
            }
            "vpn" => {
                ensure!(!self.demo, "VPN access is disabled in demo mode");
                let action = parts.get(1).map(String::as_str).unwrap_or("list");
                ensure!(
                    ["list", "status", "connect", "disconnect"].contains(&action),
                    "Use :vpn list/status/connect/disconnect UUID"
                );
                let program = self.config.nmcli.clone();
                let cancel = self.desktop_cancel.child_token();
                if action == "connect" || action == "disconnect" {
                    let id = parts
                        .get(2)
                        .context("Provide a UUID from :vpn list")?
                        .clone();
                    uuid::Uuid::parse_str(&id)?;
                    let connect = action == "connect";
                    self.tasks.spawn(async move {
                        let result =
                            ytui_services::vpn::change(&program, &id, connect, cancel).await;
                        UiEvent::Status(match result {
                            Ok(()) => "VPN action completed".into(),
                            Err(e) => ytui_services::process::diagnostic(&e.to_string()),
                        })
                    });
                } else {
                    let id = self.pending_text("VPN connections");
                    self.tasks.spawn(async move {
                        let result =
                            ytui_services::vpn::list(&program, cancel)
                                .await
                                .map(|connections| {
                                    let mut lines = vec![
                                        "Existing NetworkManager VPN/WireGuard profiles".into(),
                                        ":vpn connect UUID · :vpn disconnect UUID".into(),
                                        String::new(),
                                    ];
                                    for c in connections {
                                        lines.push(format!(
                                            "{} · {} · {}",
                                            c.name,
                                            c.kind,
                                            if c.active { "active" } else { "inactive" }
                                        ));
                                        lines.push(c.uuid);
                                        lines.push(String::new());
                                    }
                                    lines
                                });
                        UiEvent::Text { id, result }
                    });
                }
                self.status = "VPN operation running…".into();
            }
            "clipboard" if value == "on" || value == "off" => {
                let mut config = self.config.clone();
                config.clipboard_watcher = value == "on";
                self.save_config(config);
            }
            "clipboard" => {
                ensure!(
                    self.config.clipboard_watcher,
                    "Enable Clipboard watcher in :settings first"
                );
                let url = self
                    .clipboard_url
                    .clone()
                    .context("No new YouTube URL detected yet")?;
                self.modal = Some(Modal::Form(Box::new(Form::download(
                    Media {
                        title: "From clipboard".into(),
                        url,
                        ..Default::default()
                    },
                    self.config.download.clone(),
                ))));
            }
            "schedule" if value.is_empty() => {
                self.modal = Some(Modal::Form(Box::new(Form::settings(self.config.clone()))))
            }
            "schedule" => {
                let delay = schedule::delay_seconds(
                    parts
                        .get(1)
                        .context("Use :schedule 30m URL (or 2h / 90s)")?,
                )?;
                let url = normalize_media_url(parts.get(2).context("Provide a media URL")?)?;
                ensure!(!self.demo, "Scheduling is disabled in demo mode");
                let mut job = Job::new(
                    Media {
                        title: "Scheduled media".into(),
                        url,
                        ..Default::default()
                    },
                    self.config.download.clone(),
                );
                job.not_before = Some(schedule::unix_now().saturating_add(delay));
                self.snapshot.downloads.jobs.push(job);
                self.dirty = true;
                self.status = format!(
                    "Download scheduled in {delay}s · waits for the daily window if configured"
                );
            }
            "profiles" => {
                let names = ytui_services::profiles::list(&self.paths)?;
                self.status = if names.is_empty() {
                    "No saved profiles · F2 → Save settings profile".into()
                } else {
                    format!(
                        "Profiles: {} · F2 → Load settings profile",
                        names.join(", ")
                    )
                };
            }
            "profile" if value.is_empty() => self.command("profiles")?,
            "profile" => {
                let action = parts.get(1).context("Use :profile save/load/delete NAME")?;
                let name = parts.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
                ensure!(!self.demo, "Profiles are disabled in demo mode");
                match action.as_str() {
                    "save" => {
                        ytui_services::profiles::save(&self.paths, &name, &self.config)?;
                        self.status = format!("Saved profile {name}");
                    }
                    "load" => self.save_config(ytui_services::profiles::load(&self.paths, &name)?),
                    "delete" => {
                        ytui_services::profiles::delete(&self.paths, &name)?;
                        self.status = format!("Deleted profile {name}");
                    }
                    _ => anyhow::bail!("Use :profile save/load/delete NAME"),
                }
            }
            "settings" => {
                self.modal = Some(Modal::Form(Box::new(Form::settings(self.config.clone()))))
            }
            "theme" => {
                if value.is_empty() {
                    self.action(Action::Theme);
                } else {
                    if value != "system" && !PathBuf::from(&value).is_file() {
                        theme::builtin(&value)?;
                    }
                    let mut c = self.config.clone();
                    c.theme = value;
                    self.save_config(c);
                }
            }
            "history" if self.tab == 0 => self.command(&format!("downloads completed {value}"))?,
            "home" | "subscriptions" | "later" | "liked" | "library" | "history" => {
                ensure!(browse_tab != 3, "Choose YouTube or Music for this feed");
                self.tab = browse_tab;
                self.navigate(
                    browse_tab,
                    match cmd {
                        "subscriptions" => BrowseRequest::Subscriptions,
                        "later" => BrowseRequest::WatchLater,
                        "liked" => BrowseRequest::Liked,
                        "library" => BrowseRequest::Library,
                        "history" => BrowseRequest::History,
                        _ => BrowseRequest::Home,
                    },
                    false,
                );
            }
            "songs" | "albums" | "artists" | "videos" | "playlists" => {
                self.change_tab(2);
                if value.is_empty() {
                    let id = match cmd {
                        "albums" => "FEmusic_liked_albums",
                        "artists" => "FEmusic_library_corpus_artists",
                        "playlists" => "FEmusic_liked_playlists",
                        _ => "FEmusic_liked_videos",
                    };
                    ensure!(
                        self.session.is_some(),
                        "Connect your account first with :auth"
                    );
                    self.navigate(2, BrowseRequest::Collection(id.into()), false);
                } else {
                    self.navigate(
                        2,
                        BrowseRequest::MusicSearch {
                            query: value,
                            category: cmd.into(),
                        },
                        false,
                    );
                }
            }
            "details" => {
                let media = self.selected().context("Select a video first")?;
                ensure!(
                    !media.collection() && media.kind != MediaKind::Station,
                    "Select a video or song"
                );
                self.navigate(browse_tab, BrowseRequest::Details(media.id), false);
            }
            "search" => self.search(&value),
            "add" => {
                let url = normalize_media_url(&value)?;
                self.modal = Some(Modal::Form(Box::new(Form::download(
                    Media {
                        title: value,
                        url,
                        ..Default::default()
                    },
                    self.config.download.clone(),
                ))));
            }
            "convert" => {
                let path =
                    std::fs::canonicalize(&value).context("Local input file does not exist")?;
                ensure!(path.is_file(), "Choose a media file");
                self.modal = Some(Modal::Form(Box::new(Form::download(
                    Media {
                        title: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into(),
                        url: path.to_string_lossy().into(),
                        ..Default::default()
                    },
                    self.config.download.clone(),
                ))));
            }
            "channel" | "playlist" | "collection" => {
                ensure!(
                    !value.is_empty(),
                    "Provide a collection/channel/playlist ID"
                );
                let id = if cmd == "playlist" && !value.starts_with("VL") {
                    format!("VL{value}")
                } else {
                    value
                };
                self.tab = browse_tab.min(2);
                self.navigate(self.tab, BrowseRequest::Collection(id), false);
            }
            "related" => {
                let media = self.selected().context("Select a video first")?;
                self.navigate(browse_tab, BrowseRequest::Related(media.id), false);
            }
            "cliamp" => {
                self.change_tab(3);
                self.navigate(3, BrowseRequest::Cliamp, false);
            }
            "pin-country" | "unpin-country" => {
                let code = value.to_uppercase();
                ensure!(
                    code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic()),
                    "Use a two-letter country code, e.g. DE"
                );
                let mut config = self.config.clone();
                config
                    .radio_pinned_countries
                    .retain(|c| c.to_uppercase() != code);
                if cmd == "pin-country" {
                    config.radio_pinned_countries.push(code);
                }
                self.save_config(config);
            }
            "countries" => {
                let mut lines = vec![
                    "Pinned countries · :country CODE browses stations".into(),
                    String::new(),
                ];
                lines.extend(self.config.radio_pinned_countries.iter().cloned());
                lines.push(String::new());
                lines.push(":pin-country DE · :unpin-country DE".into());
                self.modal = Some(Modal::Text {
                    title: "Radio countries".into(),
                    lines,
                    offset: 0,
                });
            }
            "radio" | "country" | "tag" => {
                self.change_tab(3);
                self.navigate(
                    3,
                    match cmd {
                        "country" => BrowseRequest::RadioCountry(value),
                        "tag" => BrowseRequest::RadioTag(value),
                        _ => BrowseRequest::RadioSearch(value),
                    },
                    false,
                );
            }
            "favorites" => {
                self.change_tab(3);
                self.navigate(3, BrowseRequest::Favorites, false);
            }
            "custom" => {
                self.change_tab(3);
                self.navigate(3, BrowseRequest::Collection("custom".into()), false);
            }
            "live" => {
                let m = self.selected().context("Select a station")?;
                ensure!(m.kind == MediaKind::Station, "Select a radio station");
                self.play(m);
            }
            "auth" => {
                if value.is_empty() {
                    if self.demo {
                        self.modal = Some(Modal::Form(Box::new(Form::auth())));
                        return Ok(());
                    }
                    let paths = self.paths.clone();
                    self.modal = Some(Modal::Account {
                        profiles: Vec::new(),
                        selected: 0,
                    });
                    self.status =
                        "Discovering profile directories · cookie databases stay closed".into();
                    self.tasks.spawn(async move {
                        UiEvent::AccountProfiles(
                            tokio::task::spawn_blocking(move || auth::profiles(&paths))
                                .await
                                .unwrap_or_default(),
                        )
                    });
                } else {
                    self.import_auth(
                        parts.get(1).cloned().unwrap_or_default(),
                        parts.get(2).cloned().unwrap_or_default(),
                        String::new(),
                    );
                }
            }
            "disconnect" => {
                ensure!(!self.demo, "Account access is disabled in demo mode");
                if let Some((operation, _)) = &self.pending_operation {
                    operation.cancel_preparation();
                    self.disconnect_pending = true;
                    self.status =
                        "Disconnect queued · waiting for the current storage commit".into();
                    return Ok(());
                }
                let store = SessionStore::new(&self.paths, &self.config.auth_store);
                let operation = self.operations.account();
                self.pending_operation = Some((operation.clone(), "Disconnecting account…".into()));
                let operations = self.operations.clone();
                self.tasks.spawn(async move {
                    let result = operations
                        .commit(operation.clone(), move || store.disconnect())
                        .await;
                    UiEvent::Disconnected { operation, result }
                });
            }
            "cancel-operation" => {
                ensure!(!self.disconnect_pending, "Disconnect is already queued");
                let (operation, _) = self
                    .pending_operation
                    .as_ref()
                    .context("No account/settings operation is running")?;
                operation.cancel_preparation();
                self.status =
                    "Cancellation requested · an existing storage commit will finish safely".into();
            }
            "stopall" => {
                for j in &mut self.snapshot.downloads.jobs {
                    if j.state.active() || matches!(j.state, JobState::Queued | JobState::Paused) {
                        if let Some(t) = self.running.get(&j.id) {
                            t.cancel.cancel();
                        }
                        j.invalidate(JobState::Cancelled);
                    }
                }
                self.dirty = true;
            }
            "pauseall" => {
                for j in &mut self.snapshot.downloads.jobs {
                    if j.state.active() || j.state == JobState::Queued {
                        if let Some(t) = self.running.get(&j.id) {
                            t.cancel.cancel();
                        }
                        j.invalidate(JobState::Paused);
                    }
                }
                self.dirty = true;
            }
            "resumeall" => {
                for j in &mut self.snapshot.downloads.jobs {
                    if j.state == JobState::Paused {
                        j.invalidate(JobState::Queued);
                    }
                }
                self.dirty = true;
            }
            "clear" => {
                self.snapshot
                    .downloads
                    .jobs
                    .retain(|j| j.state != JobState::Done);
                self.dirty = true;
            }
            "import" => {
                ensure!(!self.demo, "Queue import is disabled in demo mode");
                ensure!(!value.is_empty(), "Provide an import path");
                let config = self.config.clone();
                self.tasks.spawn(async move {
                    UiEvent::Import(
                        tokio::task::spawn_blocking(move || {
                            ytui_services::queue_import::read(&PathBuf::from(value), &config)
                        })
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|r| r),
                    )
                });
            }
            "export" => {
                ensure!(!value.is_empty(), "Provide an export path");
                let data = serde_json::to_vec_pretty(&self.snapshot)?;
                ytui_services::paths::atomic_write(&PathBuf::from(value), &data)?;
                self.status = "Queue/history exported".into();
            }
            "open" => {
                ensure!(!self.offline, "Browser navigation requires online mode");
                let media = self.selected().context("Select an item")?;
                ytui_core::http_url(&media.url)?;
                let cancel = self.desktop_cancel.child_token();
                self.tasks.spawn(async move {
                    let result = ytui_services::process::run(
                        ytui_services::process::ProcessSpec::new("xdg-open", vec![media.url], 10),
                        cancel,
                        |_| {},
                    )
                    .await;
                    UiEvent::Status(if result.is_ok_and(|r| r.success) {
                        "Opened in browser".into()
                    } else {
                        "Could not open the browser".into()
                    })
                });
            }
            _ => anyhow::bail!(
                "Unknown command. Try :home :library :radio :auth :settings, or ? for help"
            ),
        }
        self.redraw = true;
        Ok(())
    }
}
pub fn key_name(key: KeyEvent) -> String {
    let name = match key.code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => "shift+tab".into(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::F(n) => format!("f{n}"),
        _ => String::new(),
    };
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        format!("ctrl+{name}")
    } else if key.modifiers.contains(KeyModifiers::ALT) {
        format!("alt+{name}")
    } else {
        name
    }
}

/// Fictional artwork for offline UI review; never fetched from a provider.
fn demo_art(palette: &Palette, index: usize, width: u32, height: u32) -> image::DynamicImage {
    let blend = |a: (u8, u8, u8), b: (u8, u8, u8), ratio: f64| {
        image::Rgb([
            (a.0 as f64 * (1.0 - ratio) + b.0 as f64 * ratio) as u8,
            (a.1 as f64 * (1.0 - ratio) + b.1 as f64 * ratio) as u8,
            (a.2 as f64 * (1.0 - ratio) + b.2 as f64 * ratio) as u8,
        ])
    };
    let image = image::RgbImage::from_fn(width.max(1), height.max(1), |x, y| {
        let x = x as f64 * 320.0 / width.max(1) as f64;
        let y = y as f64 * 180.0 / height.max(1) as f64;
        let phase = index as f64 * 0.7;
        if (x - 244.0).powi(2) + (y - 44.0).powi(2) < 20.0_f64.powi(2) {
            return blend(palette.background, palette.warning, 0.85);
        }
        if y > 139.0 + (x / 55.0 + phase).sin() * 12.0 {
            return blend(palette.background, palette.accent, 0.65);
        }
        if y > 111.0 + (x / 48.0 + phase).sin() * 25.0 {
            return blend(palette.background, palette.accent, 0.35);
        }
        blend(palette.background, palette.accent, 0.08 + y / 800.0)
    });
    image::DynamicImage::ImageRgb8(image)
}
