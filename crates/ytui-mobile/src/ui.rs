//! Touch-first, virtualized Rust UI shared by phone and tablet Android builds.
use crate::*;
use egui::{Color32, RichText, Vec2};
use std::{collections::HashSet, sync::mpsc};

#[derive(Clone, Debug)]
pub enum Effect {
    Play(Value),
    Command(Value),
    Download(Value),
    Login,
    Pick(String),
    Open(String),
    Offline(String),
}
#[derive(Clone)]
enum Kind {
    Feed {
        tab: usize,
        generation: u64,
        append: bool,
    },
    Settings,
    SettingsSaved,
    ImportedTheme,
    Theme,
    Image(String),
    Resolve {
        account_generation: u64,
        playback_generation: u64,
        queued: bool,
        media: Box<Media>,
        audio: bool,
        preview: bool,
        download: bool,
        resume: bool,
    },
    Saved,
    Queue,
    Popup(String),
    Account,
    Ignore,
}
struct Reply {
    id: u64,
    image: Option<egui::ColorImage>,
    retry_after: Option<std::time::Duration>,
    kind: Kind,
    result: std::result::Result<Value, String>,
}
#[derive(Default)]
struct Feed {
    page: Page,
    loading: bool,
    error: Option<String>,
    generation: u64,
    request: Option<BrowseRequest>,
    last_token: Option<String>,
    last_attempt: Option<(BrowseRequest, bool)>,
    failures: u8,
    retry_at: Option<std::time::Instant>,
    scroll_offset: f32,
}
pub struct MobileApp {
    engine: Arc<Engine>,
    tx: mpsc::Sender<Reply>,
    rx: mpsc::Receiver<Reply>,
    pub tab: usize,
    feeds: [Feed; 4],
    query: String,
    config: Config,
    palette: ytui_core::theme::Palette,
    pub effects: Vec<Effect>,
    pub status: String,
    pub settings_open: bool,
    pub menu_open: bool,
    pub player: Value,
    pub downloads: Value,
    player_open: bool,
    queue_open: bool,
    queue: Vec<Media>,
    queue_selection: Option<String>,
    popup: Option<(String, String)>,
    feed_menu: bool,
    network_enabled: bool,
    pending_feed: [Option<u64>; 4],
    pending_playback: Option<u64>,
    images: HashMap<String, egui::TextureHandle>,
    image_order: std::collections::VecDeque<String>,
    image_pending: HashSet<String>,
    image_failures: HashMap<String, (u8, std::time::Instant)>,
    tasks: HashMap<u64, tokio::task::AbortHandle>,
    dark: bool,
    safe_top: f32,
    safe_bottom: f32,
    phase: f32,
    last_frame: std::time::Instant,
    visual_levels: [f32; 3],
    volume: f32,
    speed: f32,
    settings_original: Option<Config>,
    settings_pending: bool,
    account_generation: u64,
    playback_generation: u64,
}
impl MobileApp {
    pub fn new(engine: Arc<Engine>, ctx: &egui::Context) -> Self {
        Self::with_network(engine, ctx, true)
    }
    fn with_network(engine: Arc<Engine>, ctx: &egui::Context, network_enabled: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            engine,
            tx,
            rx,
            tab: 1,
            feeds: Default::default(),
            query: String::new(),
            config: Config::default(),
            palette: ytui_core::theme::builtin("ytui-dark").expect("palette"),
            effects: Vec::new(),
            status: "Ready".into(),
            settings_open: false,
            menu_open: false,
            player: json!({}),
            downloads: json!([]),
            player_open: false,
            queue_open: false,
            queue: Vec::new(),
            queue_selection: None,
            popup: None,
            feed_menu: false,
            network_enabled,
            pending_feed: [None; 4],
            pending_playback: None,
            images: HashMap::new(),
            image_order: Default::default(),
            image_pending: HashSet::new(),
            image_failures: HashMap::new(),
            tasks: HashMap::new(),
            dark: true,
            safe_top: 0.0,
            safe_bottom: 0.0,
            phase: 0.0,
            last_frame: std::time::Instant::now(),
            visual_levels: [0.0; 3],
            volume: 0.7,
            speed: 1.0,
            settings_original: None,
            settings_pending: false,
            account_generation: 0,
            playback_generation: 0,
        };
        app.request(ctx, json!({"op":"settings"}), Kind::Settings);
        app.request(ctx, json!({"op":"theme","dark":true}), Kind::Theme);
        if app.network_enabled {
            app.navigate(ctx, BrowseRequest::Home, false);
        }
        app
    }
    fn request(&mut self, ctx: &egui::Context, mut data: Value, kind: Kind) -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        data["request_id"] = json!(id);
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let task = runtime().spawn(async move {
            let mut image = None;
            let mut retry_after = None;
            let result = if matches!(&kind, Kind::Image(_)) {
                let work = async {
                    let media: Media = serde_json::from_value(data["media"].clone())?;
                    let width = data["width"].as_u64().unwrap_or(240).clamp(32, 768) as u32;
                    let pixels = engine.thumbnail(&media, width, width).await?.to_rgba8();
                    let size = [pixels.width() as usize, pixels.height() as usize];
                    image = Some(egui::ColorImage::from_rgba_unmultiplied(size, &pixels));
                    Ok::<Value, anyhow::Error>(Value::Null)
                };
                work.await
                    .map_err(|e| ytui_providers::diagnostic(&format!("{e:#}")))
            } else {
                engine.request(data).await.map_err(|e| {
                    retry_after = e
                        .downcast_ref::<ytui_providers::browse::FeedRetry>()
                        .map(|r| r.after);
                    ytui_providers::diagnostic(&format!("{e:#}"))
                })
            };
            let _ = tx.send(Reply {
                id,
                kind,
                result,
                image,
                retry_after,
            });
            ctx.request_repaint();
        });
        self.tasks.insert(id, task.abort_handle());
        id
    }
    fn navigate(&mut self, ctx: &egui::Context, query: BrowseRequest, append: bool) {
        if self.tab == 0 || !self.network_enabled {
            return;
        }
        if let Some(id) = self.pending_feed[self.tab].take() {
            self.engine.cancel(id);
        }
        let tab = self.tab;
        let feed = &mut self.feeds[tab];
        if feed
            .last_attempt
            .as_ref()
            .is_none_or(|(old, _)| old != &query)
        {
            feed.failures = 0;
        }
        feed.retry_at = None;
        feed.last_attempt = Some((query.clone(), append));
        if !append {
            feed.generation += 1;
            feed.last_token = None;
            feed.request = Some(query.clone());
        }
        if let BrowseRequest::Continue(token) = &query {
            if feed.last_token.as_ref() == Some(token) {
                return;
            }
            feed.last_token = Some(token.clone());
        }
        feed.loading = true;
        feed.error = None;
        let generation = feed.generation;
        self.pending_feed[tab]=Some(self.request(ctx,json!({"op":"browse","tab":tab,"request":query,"offset":if append {self.feeds[tab].page.items.len()} else {0}}),Kind::Feed{tab,generation,append}));
    }
    pub fn change_tab(&mut self, ctx: &egui::Context, tab: usize) {
        if tab > 3 {
            return;
        }
        self.tab = tab;
        if tab > 0 && self.feeds[tab].request.is_none() {
            self.navigate(
                ctx,
                if tab == 3 {
                    BrowseRequest::Cliamp
                } else {
                    BrowseRequest::Home
                },
                false,
            );
        }
    }
    fn save_settings(&mut self, ctx: &egui::Context) {
        if self.settings_pending {
            return;
        }
        self.settings_pending = true;
        self.request(
            ctx,
            json!({"op":"save_settings","config":self.config}),
            Kind::SettingsSaved,
        );
    }
    fn resolve(
        &mut self,
        ctx: &egui::Context,
        media: Media,
        preview: bool,
        download: bool,
        resume: bool,
    ) {
        let queued = self
            .queue_selection
            .take()
            .is_some_and(|url| url == media.url)
            && !download
            && !preview;
        let audio = matches!(
            media.kind,
            ytui_core::MediaKind::Song | ytui_core::MediaKind::Station
        ) || self.config.download.audio && download;
        if download && media.live {
            self.status =
                "Live recording is not available; play or queue the station instead".into();
            return;
        }
        if media.collection() {
            if download {
                self.effects.push(Effect::Command(json!({"op":"download_playlist","request":BrowseRequest::Collection(media.id.clone()),"music":self.tab==2,"audio":audio})));
                self.status = "Preparing the complete playlist in the background…".into();
            } else {
                self.navigate(ctx, BrowseRequest::Collection(media.id.clone()), false);
            }
            return;
        }
        self.status = if download {
            "Preparing download…"
        } else {
            "Resolving playback…"
        }
        .into();
        if !download {
            self.playback_generation += 1;
            if let Some(old) = self.pending_playback.take() {
                self.engine.cancel(old);
            }
        }
        let id=self.request(ctx,json!({"op":"resolve","media":media,"audio":audio,"quality":self.config.download.quality}),Kind::Resolve{account_generation:self.account_generation,playback_generation:self.playback_generation,queued,media:Box::new(media),audio,preview,download,resume});
        if !download {
            self.pending_playback = Some(id);
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        for _ in 0..4 {
            let Ok(reply) = self.rx.try_recv() else {
                return;
            };
            self.tasks.remove(&reply.id);
            if self.pending_playback == Some(reply.id) {
                self.pending_playback = None;
            }
            if let Kind::Resolve {
                account_generation,
                playback_generation,
                download,
                ..
            } = &reply.kind
                && (*account_generation != self.account_generation
                    || (!download && *playback_generation != self.playback_generation))
            {
                continue;
            }
            match (reply.kind, reply.result) {
                (
                    Kind::Feed {
                        tab,
                        generation,
                        append,
                    },
                    result,
                ) => {
                    let feed = &mut self.feeds[tab];
                    if generation != feed.generation {
                        continue;
                    }
                    feed.loading = false;
                    self.pending_feed[tab] = None;
                    match result
                        .and_then(|v| serde_json::from_value::<Page>(v).map_err(|e| e.to_string()))
                    {
                        Ok(mut page) => {
                            feed.failures = 0;
                            feed.retry_at = None;
                            if append {
                                let known = feed
                                    .page
                                    .items
                                    .iter()
                                    .map(|m| m.url.clone())
                                    .collect::<HashSet<_>>();
                                page.items.retain(|m| !known.contains(&m.url));
                                feed.page.items.extend(page.items);
                                feed.page.continuation = page.continuation;
                                feed.page.notice = page.notice;
                            } else {
                                feed.page = page;
                            }
                            // Bounded retained window while preserving continuation for long feeds.
                            if feed.page.items.len() > 2000 {
                                let drain = feed.page.items.len() - 2000;
                                feed.page.items.drain(..drain);
                                feed.scroll_offset =
                                    (feed.scroll_offset - drain as f32 * 160.0).max(0.0);
                            }
                        }
                        Err(error) => {
                            feed.error = Some(error);
                            if let Some(after) = reply.retry_after
                                && feed.failures < 3
                            {
                                feed.failures += 1;
                                feed.retry_at = Some(
                                    std::time::Instant::now()
                                        + after.max(std::time::Duration::from_secs(
                                            1_u64 << feed.failures,
                                        )),
                                );
                            }
                            feed.last_token = None;
                        }
                    }
                }
                (Kind::Settings, Ok(value)) => {
                    if let Ok(config) = serde_json::from_value(value) {
                        self.config = config;
                    }
                }
                (Kind::SettingsSaved, Err(error)) => {
                    self.settings_pending = false;
                    self.settings_open = true;
                    self.status = error;
                }
                (Kind::SettingsSaved, Ok(_)) => {
                    self.settings_pending = false;
                    self.settings_original = None;
                    self.request(ctx, json!({"op":"theme","dark":self.dark}), Kind::Theme);
                    self.effects.push(Effect::Command(
                        json!({"op":"settings","config":self.config}),
                    ));
                    self.status = "Settings saved".into();
                }
                (Kind::ImportedTheme, Ok(value)) => {
                    if let Ok(palette) = serde_json::from_value(value) {
                        self.palette = palette;
                        self.theme_bars();
                    }
                    self.request(ctx, json!({"op":"settings"}), Kind::Settings);
                    self.settings_original = None;
                }
                (Kind::Theme, Ok(value)) => {
                    if let Ok(palette) = serde_json::from_value(value) {
                        self.palette = palette;
                        self.theme_bars();
                    }
                }
                (Kind::Image(url), _result) => {
                    self.image_pending.remove(&url);
                    if let Some(image) = reply.image {
                        self.image_failures.remove(&url);
                        let texture =
                            ctx.load_texture(url.clone(), image, egui::TextureOptions::LINEAR);
                        let bytes = texture.size()[0] * texture.size()[1] * 4;
                        while self.images.len() >= 40
                            || self
                                .images
                                .values()
                                .map(|image| image.size()[0] * image.size()[1] * 4)
                                .sum::<usize>()
                                + bytes
                                > 24 * 1024 * 1024
                        {
                            let Some(old) = self.image_order.pop_front() else {
                                break;
                            };
                            self.images.remove(&old);
                        }
                        self.images.insert(url.clone(), texture);
                        self.image_order.push_back(url);
                    } else {
                        let attempts = self
                            .image_failures
                            .get(&url)
                            .map_or(1, |(n, _)| n.saturating_add(1));
                        if self.image_failures.len() >= 512
                            && let Some(old) = self.image_failures.keys().next().cloned()
                        {
                            self.image_failures.remove(&old);
                        }
                        self.image_failures.insert(
                            url,
                            (
                                attempts,
                                std::time::Instant::now()
                                    + std::time::Duration::from_secs(
                                        2_u64.pow(attempts.min(5) as u32),
                                    ),
                            ),
                        );
                    }
                }
                (
                    Kind::Resolve {
                        media,
                        audio,
                        preview,
                        download,
                        resume,
                        queued,
                        ..
                    },
                    Ok(stream),
                ) => {
                    if !download
                        && (!audio || self.config.music_player_style.ends_with("-fullscreen"))
                    {
                        self.player_open = true;
                    }
                    let request = json!({"media":media,"stream":stream,"audio":audio,"preview":preview,"resume":resume,"queued":queued});
                    self.effects.push(if download {
                        Effect::Download(request)
                    } else {
                        Effect::Play(request)
                    });
                    self.status = if download {
                        "Download queued"
                    } else {
                        "Starting playback"
                    }
                    .into();
                }
                (Kind::Queue, Ok(value)) => {
                    self.queue = serde_json::from_value(value).unwrap_or_default();
                    self.queue_open = true;
                }
                (Kind::Saved, Ok(value)) => {
                    if let Ok(items) = serde_json::from_value(value) {
                        self.feeds[self.tab].page = Page {
                            title: "Saved locally".into(),
                            items,
                            ..Default::default()
                        };
                    }
                }
                (Kind::Popup(title), Ok(value)) => {
                    self.popup = Some((
                        title.clone(),
                        if title == "Lyrics" {
                            lyric_text(&value)
                        } else if let Some(s) = value.as_str() {
                            s.into()
                        } else {
                            serde_json::to_string_pretty(&value).unwrap_or_default()
                        },
                    ));
                }
                (Kind::Account, Ok(_)) => {
                    self.account_generation += 1;
                    self.playback_generation += 1;
                    for tab in 1..=2 {
                        if let Some(id) = self.pending_feed[tab].take() {
                            self.engine.cancel(id);
                        }
                        let feed = &mut self.feeds[tab];
                        feed.generation += 1;
                        feed.page = Page::default();
                        feed.request = None;
                        feed.loading = false;
                        feed.error = None;
                    }
                    self.status = "Account updated".into();
                    if matches!(self.tab, 1 | 2) {
                        self.navigate(ctx, BrowseRequest::Home, false);
                    }
                }
                (_, Err(error)) => {
                    self.status = error;
                }
                _ => {}
            }
        }
        ctx.request_repaint();
    }
    pub fn platform_status(&mut self, ctx: &egui::Context, value: Value) {
        self.safe_top = value["inset_top"].as_f64().unwrap_or(0.0) as f32 / ctx.pixels_per_point();
        self.safe_bottom =
            value["inset_bottom"].as_f64().unwrap_or(0.0) as f32 / ctx.pixels_per_point();
        if let Some(dark) = value["dark"].as_bool()
            && dark != self.dark
        {
            self.dark = dark;
            if self.config.theme == "system" {
                self.request(ctx, json!({"op":"theme","dark":dark}), Kind::Theme);
            }
        }
        if let Some(downloads) = value.get("downloads") {
            self.downloads = downloads.clone();
        }
        if let Some(player) = value.get("player") {
            if let Some(v) = player["volume"].as_f64() {
                self.volume = v as f32;
            }
            if let Some(v) = player["speed"].as_f64() {
                self.speed = v as f32;
            }
            self.player = player.clone();
        }
        if let Some(events) = value["events"].as_array() {
            for event in events {
                match event["kind"].as_str().unwrap_or_default() {
                    "cookies" => {
                        self.account_generation += 1;
                        self.playback_generation += 1;
                        self.request(
                            ctx,
                            json!({"op":"import_cookies","header":event["text"]}),
                            Kind::Account,
                        );
                    }
                    "theme_file" => {
                        self.request(
                            ctx,
                            json!({"op":"import_theme","text":event["text"]}),
                            Kind::ImportedTheme,
                        );
                    }
                    "cookie_file" => {
                        self.account_generation += 1;
                        self.playback_generation += 1;
                        self.request(
                            ctx,
                            json!({"op":"import_cookie_file","text":event["text"]}),
                            Kind::Account,
                        );
                    }
                    "show_player" => {
                        self.player_open = true;
                    }
                    "shared" => {
                        self.query = event["text"].as_str().unwrap_or_default().trim().to_owned();
                    }
                    "tracks" => {
                        self.popup = Some((
                            "Tracks".into(),
                            event["text"].as_str().unwrap_or_default().into(),
                        ));
                    }
                    "back" => {
                        if self.settings_open {
                            self.settings_open = false;
                            if let Some(original) = self.settings_original.take() {
                                self.config = original;
                            }
                        } else if self.menu_open {
                            self.menu_open = false;
                        } else if self.popup.is_some() {
                            self.popup = None;
                        } else if self.queue_open {
                            self.queue_open = false;
                        } else if self.player_open {
                            self.player_open = false;
                        } else {
                            self.effects
                                .push(Effect::Command(json!({"op":"background"})));
                        }
                    }
                    "error" => {
                        self.status = event["text"]
                            .as_str()
                            .unwrap_or("Android operation failed")
                            .into()
                    }
                    _ => {}
                }
            }
        }
    }
    pub fn frame(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        if self.feeds[self.tab]
            .retry_at
            .is_some_and(|at| std::time::Instant::now() >= at)
        {
            self.feeds[self.tab].retry_at = None;
            if let Some((query, append)) = self.feeds[self.tab].last_attempt.clone() {
                self.navigate(ctx, query, append);
            }
        }
        let now = std::time::Instant::now();
        let playing = self.player["playing"].as_bool().unwrap_or(false);
        let buffering = self.player["buffering"].as_bool().unwrap_or(false);
        let animate = playing
            && !buffering
            && !self.config.reduced_motion
            && matches!(self.tab, 2 | 3)
            && self.config.music_player_style != "classic";
        if animate {
            self.phase = (self.phase
                + (now - self.last_frame).as_secs_f32() * std::f32::consts::TAU / 4.0)
                % std::f32::consts::TAU;
            self.visual_levels = [
                self.player["left"].as_f64().unwrap_or(0.0) as f32,
                self.player["right"].as_f64().unwrap_or(0.0) as f32,
                self.player["peak"].as_f64().unwrap_or(0.0) as f32,
            ];
        }
        self.last_frame = now;
        let mut style = (*ctx.style()).clone();
        style.spacing.interact_size = Vec2::new(48.0, 48.0);
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::monospace(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::monospace(15.0));
        for font in style.text_styles.values_mut() {
            font.family = egui::FontFamily::Monospace;
        }
        style.visuals = if self.palette.light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        style.visuals.panel_fill = c(self.palette.background);
        style.visuals.window_fill = c(self.palette.surface);
        style.visuals.override_text_color = Some(c(self.palette.text));
        style.visuals.selection.bg_fill = c(self.palette.selection);
        style.visuals.widgets.inactive.bg_fill = c(self.palette.surface);
        style.visuals.widgets.active.bg_fill = c(self.palette.selection);
        ctx.set_style(style);
        let width = ctx.content_rect().width();
        let tablet = width >= 700.0;
        if self.safe_top > 0.0 {
            egui::TopBottomPanel::top("safe-top")
                .exact_height(self.safe_top)
                .show(ctx, |_| {});
        }
        if self.safe_bottom > 0.0 {
            egui::TopBottomPanel::bottom("safe-bottom")
                .exact_height(self.safe_bottom)
                .show(ctx, |_| {});
        }
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("ytui")
                        .monospace()
                        .strong()
                        .color(c(self.palette.accent)),
                );
                ui.label(["Files", "YouTube", "Music", "Radio"][self.tab]);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Menu").clicked() {
                        self.menu_open = true;
                    }
                    if ui.button("Settings").clicked() {
                        self.settings_open = true;
                    }
                });
            });
        });
        egui::TopBottomPanel::bottom("navigation").show(ctx, |ui| {
            ui.columns(4, |columns| {
                for (tab, label) in ["Files", "YouTube", "Music", "Radio"].iter().enumerate() {
                    if columns[tab]
                        .add_sized(
                            [columns[tab].available_width(), 48.0],
                            egui::Button::new(*label).selected(self.tab == tab),
                        )
                        .clicked()
                    {
                        self.change_tab(ctx, tab);
                    }
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(&self.status)
                        .small()
                        .color(c(self.palette.muted)),
                )
                .truncate(),
            );
        });
        let has_media = self.player["title"].as_str().is_some_and(|s| !s.is_empty());
        if has_media {
            egui::TopBottomPanel::bottom("mini-player").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                        self.command("pause");
                    }
                    let title_width = (ui.available_width() - 72.0).max(40.0);
                    if ui
                        .add_sized(
                            [title_width, 48.0],
                            egui::Button::new(
                                self.player["title"].as_str().unwrap_or("Now playing"),
                            )
                            .truncate(),
                        )
                        .clicked()
                    {
                        self.player_open = true;
                    }
                    if ui.button("Next").clicked() {
                        self.command("next");
                    }
                });
                let progress = progress(&self.player);
                ui.add(egui::ProgressBar::new(progress).fill(c(self.palette.accent)));
            });
        }
        if !tablet
            && has_media
            && matches!(self.tab, 2 | 3)
            && self.config.music_player_style != "classic"
            && !self.config.music_player_style.ends_with("-fullscreen")
        {
            egui::TopBottomPanel::bottom("music-presentation")
                .default_height(160.0)
                .min_height(96.0)
                .max_height(240.0)
                .resizable(true)
                .show(ctx, |ui| {
                    self.visualizer(ui, ui.available_height().max(80.0));
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if tablet && matches!(self.tab, 2 | 3) {
                ui.columns(2, |cols| {
                    self.catalog(ctx, &mut cols[0]);
                    self.player_panel(ctx, &mut cols[1], false);
                });
            } else {
                self.catalog(ctx, ui);
            }
        });
        if self.settings_open {
            self.settings(ctx);
        }
        if self.menu_open {
            self.menu(ctx);
        }
        if self.feed_menu {
            self.feeds(ctx);
        }
        if self.player_open {
            let mut open = true;
            egui::Window::new("Now playing")
                .open(&mut open)
                .default_pos(ctx.content_rect().left_top() + Vec2::splat(8.0))
                .default_size((width - 16.0, ctx.content_rect().height() - 32.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    self.player_panel(ctx, ui, true);
                });
            self.player_open = open;
        }
        if self.queue_open {
            let mut open = true;
            egui::Window::new("Playback queue")
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(500.0)
                        .show(ui, |ui| {
                            for (i, media) in self.queue.clone().iter().enumerate() {
                                ui.horizontal(|ui| {
                                    if ui.button(&media.title).clicked() {
                                        self.queue_selection = Some(media.url.clone());
                                        self.resolve(ctx, media.clone(), false, false, false);
                                        self.queue_open = false;
                                    }
                                    if ui.button("Remove").clicked() {
                                        self.request(
                                            ctx,
                                            json!({"op":"queue_remove","index":i}),
                                            Kind::Ignore,
                                        );
                                        self.queue.remove(i);
                                    }
                                });
                            }
                        });
                });
            self.queue_open = open;
        }
        if let Some((title, text)) = self.popup.clone() {
            let mut open = true;
            egui::Window::new(title).open(&mut open).show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(500.0)
                    .show(ui, |ui| {
                        ui.label(text);
                    });
            });
            if !open {
                self.popup = None;
            }
        }
        ctx.request_repaint_after(
            if animate && (self.player_open || tablet && matches!(self.tab, 2 | 3)) {
                std::time::Duration::from_millis(33)
            } else if has_media {
                std::time::Duration::from_millis(250)
            } else {
                std::time::Duration::from_secs(1)
            },
        );
    }
    fn theme_bars(&mut self) {
        #[cfg(target_os = "android")]
        self.effects.push(Effect::Command(
            json!({"op":"theme","light":self.palette.light,"background":self.palette.background}),
        ));
    }
    fn command(&mut self, op: &str) {
        self.effects.push(Effect::Command(json!({"op":op})));
    }
    fn catalog(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.tab == 0 {
            self.download_list(ui);
            return;
        }
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 170.0).max(48.0);
            let response = ui.add_sized(
                [width, 48.0],
                egui::TextEdit::singleline(&mut self.query)
                    .desired_width(width)
                    .hint_text("Search or URL"),
            );
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Search").clicked() || enter {
                let query = self.query.trim().to_owned();
                if ytui_core::http_url(&query).is_ok() {
                    let kind = if self.tab == 3 {
                        ytui_core::MediaKind::Station
                    } else if self.tab == 2 {
                        ytui_core::MediaKind::Song
                    } else {
                        ytui_core::MediaKind::Video
                    };
                    self.resolve(
                        ctx,
                        Media {
                            url: query.clone(),
                            title: query,
                            kind,
                            live: self.tab == 3,
                            ..Default::default()
                        },
                        false,
                        false,
                        false,
                    );
                } else if !query.is_empty() {
                    self.navigate(
                        ctx,
                        if self.tab == 3 {
                            BrowseRequest::RadioSearch(query)
                        } else {
                            BrowseRequest::Search(query)
                        },
                        false,
                    );
                }
            }
            if ui.button("Feeds").clicked() {
                self.feed_menu = true;
            }
        });
        let categories = self.feeds[self.tab].page.categories.clone();
        egui::ScrollArea::horizontal()
            .id_salt("categories")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for category in categories {
                        if ui.button(category.title).clicked() {
                            self.navigate(ctx, category.request, false);
                        }
                    }
                });
            });
        ui.label(RichText::new(&self.feeds[self.tab].page.title).color(c(self.palette.accent)));
        if let Some(error) = self.feeds[self.tab].error.clone() {
            ui.colored_label(
                c(self.palette.error),
                "Could not load this feed. Check your connection or choose another source.",
            );
            ui.collapsing("Connection details", |ui| {
                ui.label(error);
            });
            let retrying = self.feeds[self.tab].retry_at.is_some();
            if retrying {
                ui.label("Reconnecting automatically after the service cooldown…");
            }
            if ui
                .add_enabled(!retrying, egui::Button::new("Retry"))
                .clicked()
                && let Some(request) = self.feeds[self.tab].request.clone()
            {
                self.feeds[self.tab].failures = 0;
                self.navigate(ctx, request, false);
            }
        }
        if self.feeds[self.tab].loading {
            ui.spinner();
        }
        let count = self.feeds[self.tab].page.items.len();
        let mut near_end = false;
        let scroll = egui::ScrollArea::vertical()
            .id_salt(("feed", self.tab, self.feeds[self.tab].generation))
            .vertical_scroll_offset(self.feeds[self.tab].scroll_offset)
            .max_height((ui.available_height() - 32.0).max(80.0))
            .auto_shrink([false, false])
            .show_rows(ui, 152.0, count, |ui, range| {
                near_end = range.end.saturating_add(6) >= count;
                for index in range {
                    let media = self.feeds[self.tab].page.items[index].clone();
                    self.media_card(ctx, ui, &media);
                }
            });
        self.feeds[self.tab].scroll_offset = scroll.state.offset.y;
        if near_end
            && !self.feeds[self.tab].loading
            && self.feeds[self.tab].error.is_none()
            && let Some(token) = self.feeds[self.tab].page.continuation.clone()
        {
            self.navigate(ctx, BrowseRequest::Continue(token), true);
        }
        if count == 0 && !self.feeds[self.tab].loading && self.feeds[self.tab].error.is_none() {
            ui.label("No items returned. Choose another feed or search.");
        }
    }
    fn media_card(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, media: &Media) {
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            Vec2::new(width, 152.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::Frame::new()
                    .fill(c(self.palette.surface))
                    .stroke(egui::Stroke::new(1.0_f32, c(self.palette.selection)))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_min_width((width - 16.0).max(1.0));
                        ui.horizontal_top(|ui| {
                            if self.config.thumbnails && !media.thumbnail.is_empty() {
                                if let Some(image) = self.images.get(&media.thumbnail) {
                                    ui.add(
                                        egui::Image::new(image)
                                            .fit_to_exact_size(Vec2::splat(80.0)),
                                    );
                                } else {
                                    ui.allocate_space(Vec2::splat(80.0));
                                    if self.image_pending.len() < 8
                                        && self
                                            .image_failures
                                            .get(&media.thumbnail)
                                            .is_none_or(|(_, at)| std::time::Instant::now() >= *at)
                                        && self.image_pending.insert(media.thumbnail.clone())
                                    {
                                        self.request(
                                            ctx,
                                            json!({"op":"image","media":media}),
                                            Kind::Image(media.thumbnail.clone()),
                                        );
                                    }
                                }
                            }
                            ui.vertical(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(RichText::new(&media.title).strong())
                                            .truncate(),
                                    )
                                    .clicked()
                                {
                                    self.resolve(ctx, media.clone(), false, false, false);
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{} · {}",
                                            media.creator,
                                            media.label()
                                        ))
                                        .small(),
                                    )
                                    .truncate(),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("Preview").clicked() {
                                        self.resolve(ctx, media.clone(), true, false, false);
                                    }
                                    if ui.button("+").clicked() {
                                        self.status="Adding to the queue…".into();
                                        self.request(ctx,if media.collection(){json!({"op":"enqueue_collection","request":BrowseRequest::Collection(media.id.clone()),"music":self.tab==2})}else{json!({"op":"enqueue","media":media})},Kind::Ignore);
                                    }
                                    ui.menu_button("More", |ui| {
                                        if ui.button("Download").clicked() {
                                            self.resolve(ctx, media.clone(), false, true, false);
                                            ui.close();
                                        }
                                        if ui.button("Save / favorite").clicked() {
                                            self.request(
                                                ctx,
                                                json!({"op":"save_media","media":media}),
                                                Kind::Ignore,
                                            );
                                            ui.close();
                                        }
                                        if ui.button("Resume").clicked() {
                                            self.resolve(ctx, media.clone(), false, false, true);
                                            ui.close();
                                        }
                                        if ui.button("Open in browser").clicked() {
                                            self.effects.push(Effect::Open(media.url.clone()));
                                            ui.close();
                                        }
                                    });
                                });
                            });
                        });
                    });
            },
        );
    }
    fn player_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, expanded: bool) {
        let title = self.player["title"]
            .as_str()
            .unwrap_or("Choose a song or station")
            .to_owned();
        ui.heading(title);
        if let Some(error) = self.player["error"].as_str() {
            ui.colored_label(c(self.palette.error), error);
        }
        if self.player["video"].as_bool().unwrap_or(false)
            && self.player["title"].as_str().is_some()
        {
            let w = ui.available_width();
            let width = self.player["video_width"].as_f64().unwrap_or(0.0);
            let height = self.player["video_height"].as_f64().unwrap_or(0.0);
            let aspect = if width > 0.0 && height > 0.0 {
                (width / height).clamp(0.25, 4.0) as f32
            } else {
                16.0 / 9.0
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, w / aspect), egui::Sense::click());
            #[cfg(target_os = "android")]
            crate::platform::paint_video(ui, rect);
            #[cfg(not(target_os = "android"))]
            ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
            if let Some(text) = self.player["captions"].as_str().filter(|s| !s.is_empty()) {
                ui.painter().text(
                    rect.center_bottom() - egui::vec2(0.0, 12.0),
                    egui::Align2::CENTER_BOTTOM,
                    text,
                    egui::FontId::proportional(18.0),
                    Color32::WHITE,
                );
            }
        } else if matches!(self.tab, 2 | 3) {
            self.visualizer(ui, if expanded { 280.0 } else { 220.0 });
        }
        let duration = self.player["duration"].as_f64().unwrap_or(0.0).max(0.0);
        let mut position = self.player["position"].as_f64().unwrap_or(0.0);
        if duration > 0.0
            && ui
                .add(egui::Slider::new(&mut position, 0.0..=duration).text("seconds"))
                .drag_stopped()
        {
            self.effects
                .push(Effect::Command(json!({"op":"seek","seconds":position})));
        }
        ui.horizontal_wrapped(|ui| {
            for (label, op) in [
                ("Previous", "previous"),
                ("-10s", "back"),
                ("Play / pause", "pause"),
                ("+10s", "forward"),
                ("Next", "next"),
            ] {
                if ui.button(label).clicked() {
                    self.command(op);
                }
            }
        });
        if ui
            .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text("Volume"))
            .changed()
        {
            self.effects
                .push(Effect::Command(json!({"op":"volume","value":self.volume})));
        }
        if ui
            .add(egui::Slider::new(&mut self.speed, 0.25..=4.0).text("Speed"))
            .changed()
        {
            self.effects
                .push(Effect::Command(json!({"op":"speed","value":self.speed})));
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Queue").clicked() {
                self.request(ctx, json!({"op":"queue"}), Kind::Queue);
            }
            if ui.button("Shuffle").clicked() {
                self.command("shuffle");
            }
            if ui.button("Repeat").clicked() {
                self.command("repeat");
            }
            ui.menu_button("Tracks / captions", |ui| {
                for track in self.player["tracks"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    if ui
                        .button(track["label"].as_str().unwrap_or("Track"))
                        .clicked()
                    {
                        self.effects.push(Effect::Command(
                            json!({"op":"track","group":track["group"],"index":track["index"]}),
                        ));
                        ui.close();
                    }
                }
                if ui.button("Toggle captions").clicked() {
                    self.command("captions_toggle");
                    ui.close();
                }
            });
            if ui.button("Stop").clicked() {
                self.command("stop");
            }
        });
        if matches!(self.tab, 2 | 3) {
            let current = self.config.music_player_style.clone();
            egui::ComboBox::from_label("Player style")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for style in ytui_core::MUSIC_PLAYER_STYLES {
                        ui.selectable_value(
                            &mut self.config.music_player_style,
                            style.to_string(),
                            *style,
                        );
                    }
                });
            if current != self.config.music_player_style {
                self.save_settings(ctx);
            }
        }
        if ui.button("Replay skipped segment").clicked() {
            self.command("sponsor_replay");
        }
    }
    fn visualizer(&mut self, ui: &mut egui::Ui, height: f32) {
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let p = ui.painter_at(rect);
        let center = rect.center();
        let radius = rect.width().min(rect.height()) * 0.43;
        let style = self.config.music_player_style.as_str();
        if style.starts_with("vu") {
            for channel in 0..2 {
                let x = rect.left() + rect.width() * (0.15 + channel as f32 * 0.5);
                let w = rect.width() * 0.2;
                for led in 0..18 {
                    let y = rect.bottom() - 12.0 - led as f32 * (height - 24.0) / 18.0;
                    let fill = if (led as f32) / 18.0 <= self.visual_levels[channel] {
                        if led > 15 {
                            c(self.palette.warning)
                        } else {
                            c(self.palette.accent)
                        }
                    } else {
                        c(self.palette.surface)
                    };
                    p.rect_filled(
                        egui::Rect::from_min_size(egui::pos2(x, y - 8.0), Vec2::new(w, 6.0)),
                        1.0,
                        fill,
                    );
                }
            }
        } else if style.starts_with("orbit") {
            for ring in 0..4 {
                p.circle_stroke(
                    center,
                    radius * (0.2 + ring as f32 * 0.18 + self.visual_levels[0] * 0.12),
                    egui::Stroke::new(
                        2.0_f32,
                        c(if ring % 2 == 0 {
                            self.palette.accent
                        } else {
                            self.palette.success
                        }),
                    ),
                );
            }
            let pos =
                center + Vec2::angled(self.phase) * radius * (0.5 + self.visual_levels[2] * 0.3);
            p.circle_filled(pos, 5.0, c(self.palette.warning));
        } else {
            let classic = style == "classic";
            let side = if classic {
                rect.width().min(rect.height() - 12.0)
            } else {
                (rect.width() / 1.55).min(rect.height() - 12.0)
            };
            let x = rect.center().x - side * if classic { 0.5 } else { 0.775 };
            let y = rect.center().y - side * 0.5;
            let center = egui::pos2(x + side * 1.04, rect.center().y);
            let radius = side * 0.5;
            if !classic {
                p.circle_filled(center, radius, Color32::from_rgb(25, 25, 29));
                for ring in 1..9 {
                    p.circle_stroke(
                        center,
                        radius * ring as f32 / 9.0,
                        egui::Stroke::new(1.0_f32, Color32::from_gray(45)),
                    );
                }
                p.circle_filled(center, radius * 0.27, c(self.palette.accent));
                p.line_segment(
                    [center, center + Vec2::angled(self.phase) * radius * 0.25],
                    egui::Stroke::new(3.0_f32, Color32::BLACK),
                );
            }
            let sleeve = egui::Rect::from_min_size(egui::pos2(x, y), Vec2::splat(side));
            if let Some(media) = self
                .player
                .get("media")
                .and_then(|v| serde_json::from_value::<Media>(v.clone()).ok())
            {
                let pixels = (side * ui.ctx().pixels_per_point())
                    .ceil()
                    .clamp(240.0, 768.0) as u32;
                let pixels = pixels.div_ceil(64) * 64;
                let art_key = format!("{}#art-{pixels}", media.thumbnail);
                if let Some(image) = self.images.get(&art_key) {
                    p.image(
                        image.id(),
                        sleeve,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    p.rect_filled(sleeve, 2.0, c(self.palette.surface));
                    if !media.thumbnail.is_empty()
                        && self.image_pending.len() < 8
                        && self
                            .image_failures
                            .get(&art_key)
                            .is_none_or(|(_, at)| std::time::Instant::now() >= *at)
                        && self.image_pending.insert(art_key.clone())
                    {
                        self.request(
                            ui.ctx(),
                            json!({"op":"image","media":media,"width":pixels}),
                            Kind::Image(art_key),
                        );
                    }
                }
            }
        }
    }
    fn settings(&mut self, ctx: &egui::Context) {
        self.settings_original
            .get_or_insert_with(|| self.config.clone());
        let mut saved = false;
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .vscroll(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label("Appearance");
                egui::ComboBox::from_label("Theme")
                    .selected_text(&self.config.theme)
                    .show_ui(ui, |ui| {
                        for theme in ytui_core::theme::THEMES {
                            ui.selectable_value(&mut self.config.theme, theme.to_string(), *theme);
                        }
                    });
                if ui.button("Import Omarchy / Caelestia palette").clicked() {
                    self.effects.push(Effect::Pick("theme".into()));
                }
                ui.checkbox(&mut self.config.thumbnails, "Cached thumbnails");
                ui.checkbox(&mut self.config.reduced_motion, "Reduced motion");
                ui.checkbox(
                    &mut self.config.playback_history,
                    "Save local resume positions",
                );
                egui::ComboBox::from_label("Music player")
                    .selected_text(&self.config.music_player_style)
                    .show_ui(ui, |ui| {
                        for mode in ytui_core::MUSIC_PLAYER_STYLES {
                            ui.selectable_value(
                                &mut self.config.music_player_style,
                                mode.to_string(),
                                *mode,
                            );
                        }
                    });
                ui.separator();
                ui.label("Playback / downloads");
                egui::ComboBox::from_label("Quality")
                    .selected_text(&self.config.download.quality)
                    .show_ui(ui, |ui| {
                        for q in ["best", "1080", "720", "480", "360"] {
                            ui.selectable_value(&mut self.config.download.quality, q.into(), q);
                        }
                    });
                ui.checkbox(&mut self.config.download.audio, "Audio-only downloads");
                egui::ComboBox::from_label("SponsorBlock")
                    .selected_text(&self.config.playback_sponsorblock)
                    .show_ui(ui, |ui| {
                        for mode in ["skip", "mark", "off"] {
                            ui.selectable_value(
                                &mut self.config.playback_sponsorblock,
                                mode.into(),
                                mode,
                            );
                        }
                    });
                ui.label("Network proxy (optional)");
                let mut proxy = self.config.proxy.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut proxy).changed() {
                    self.config.proxy = if proxy.trim().is_empty() {
                        None
                    } else {
                        Some(proxy)
                    };
                }
                if ui
                    .add_enabled(!self.settings_pending, egui::Button::new("Save settings"))
                    .clicked()
                {
                    saved = true;
                    self.save_settings(ctx);
                    self.settings_open = false;
                }
            });
        self.settings_open &= open;
        if !self.settings_open
            && !saved
            && let Some(original) = self.settings_original.take()
        {
            self.config = original;
        }
    }
    fn feeds(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Feeds and library")
            .open(&mut open)
            .show(ctx, |ui| {
                let choices = if self.tab == 3 {
                    vec![
                        ("Cliamp channels", BrowseRequest::Cliamp),
                        ("Saved stations", BrowseRequest::Favorites),
                        (
                            "Country: Germany",
                            BrowseRequest::RadioCountry("Germany".into()),
                        ),
                    ]
                } else {
                    vec![
                        ("For you", BrowseRequest::Home),
                        ("Subscriptions", BrowseRequest::Subscriptions),
                        ("Liked", BrowseRequest::Liked),
                        ("Watch later", BrowseRequest::WatchLater),
                        ("Library", BrowseRequest::Library),
                        (
                            "Saved locally",
                            BrowseRequest::SavedMedia {
                                query: String::new(),
                            },
                        ),
                    ]
                };
                for (label, request) in choices {
                    if ui.button(label).clicked() {
                        self.navigate(ctx, request, false);
                        self.feed_menu = false;
                    }
                }
            });
        self.feed_menu &= open;
    }
    fn menu(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Actions").open(&mut open).show(ctx,|ui|{
            if ui.button("Look up lyrics for this song").clicked()&& let Some(media)=self.player.get("media"){self.request(ctx,json!({"op":"lyrics","media":media}),Kind::Popup("Lyrics".into()));}
            if ui.button("Sleep timer: 30 minutes").clicked(){self.effects.push(Effect::Command(json!({"op":"sleep","seconds":1800})));}

        if ui.button("Connect YouTube account").clicked(){self.effects.push(Effect::Login);self.menu_open=false;}
        if ui.button("Import cookie file").clicked(){self.effects.push(Effect::Pick("cookies".into()));}
        if ui.button("Disconnect account").clicked(){self.account_generation+=1; self.playback_generation+=1; self.request(ctx,json!({"op":"disconnect"}),Kind::Account);self.effects.push(Effect::Command(json!({"op":"disconnect"})));}
        if ui.button("Saved media").clicked(){self.request(ctx,json!({"op":"saved"}),Kind::Saved);}
        if ui.button("Playback queue").clicked(){self.request(ctx,json!({"op":"queue"}),Kind::Queue);}
        if ui.button("Refresh feed").clicked()&& let Some(req)=self.feeds[self.tab].request.clone(){self.navigate(ctx,req,false);}
        if ui.button("About / mobile parity").clicked(){self.popup=Some(("About ytui Android".into(),"Rust UI and engine · Media3 playback\nNative YouTube signature challenges may still fail.\nAndroid cannot access other browsers' private cookies.\nLinux desktop services and external cliamp are not Android services.\nSee docs/android/PARITY.md for platform differences and qualification gates.".into()));}
    });
        self.menu_open &= open;
    }
    fn download_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Offline downloads");
        let items = self.downloads.as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            ui.label("Download a video, song or playlist from its More menu.");
        }
        egui::ScrollArea::vertical().show_rows(ui, 168.0, items.len(), |ui, range| {
            for index in range {
                let item = &items[index];
                ui.group(|ui| {
                    ui.set_min_height(152.0);
                    ui.add(
                        egui::Label::new(item["title"].as_str().unwrap_or("Download")).truncate(),
                    );
                    let state = item["state"].as_str().unwrap_or("Queued");
                    ui.add(egui::Label::new(state).truncate());
                    ui.add(egui::ProgressBar::new(
                        item["progress"].as_f64().unwrap_or(0.0) as f32,
                    ));
                    let id = item["id"].as_str().unwrap_or("");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(state == "Complete", egui::Button::new("Play offline"))
                            .clicked()
                        {
                            self.effects.push(Effect::Offline(id.into()));
                        }
                        ui.menu_button("Actions", |ui| {
                            if ui.button("Pause / retry").clicked() {
                                self.effects
                                    .push(Effect::Command(json!({"op":"download_toggle","id":id})));
                                ui.close();
                            }
                            if ui.button("Remove").clicked() {
                                self.effects
                                    .push(Effect::Command(json!({"op":"download_remove","id":id})));
                                ui.close();
                            }
                        });
                    });
                });
            }
        });
    }
}
fn c(rgb: (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}
fn progress(player: &Value) -> f32 {
    let duration = player["duration"].as_f64().unwrap_or(0.0);
    if duration > 0.0 {
        (player["position"].as_f64().unwrap_or(0.0) / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

fn lyric_text(value: &Value) -> String {
    let Some(tracks) = value.as_array() else {
        return "No lyrics returned".into();
    };
    if tracks.is_empty() {
        return "No lyrics found".into();
    }
    tracks
        .iter()
        .take(5)
        .map(|track| {
            let heading = format!(
                "{} — {}",
                track["title"].as_str().unwrap_or("Lyrics"),
                track["artist"].as_str().unwrap_or("")
            );
            let lines = track["lines"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|line| line["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            format!("{heading}\n\n{lines}")
        })
        .collect::<Vec<_>>()
        .join("\n\n────────\n\n")
}

impl Drop for MobileApp {
    fn drop(&mut self) {
        for task in self.tasks.values() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_app() -> (MobileApp, egui::Context, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let engine =
            Arc::new(Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap());
        let mut app = MobileApp::with_network(engine, &ctx, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !app.tasks.is_empty() {
            app.poll(&ctx);
            assert!(
                std::time::Instant::now() < deadline,
                "Local startup requests stalled"
            );
            std::thread::yield_now();
        }
        (app, ctx, root)
    }
    fn deliver(
        app: &MobileApp,
        kind: Kind,
        result: std::result::Result<Value, String>,
        retry_after: Option<std::time::Duration>,
    ) {
        app.tx
            .send(Reply {
                id: u64::MAX,
                image: None,
                retry_after,
                kind,
                result,
            })
            .unwrap();
    }
    #[test]
    fn stale_playback_and_account_replies_cannot_start_media() {
        let (mut app, ctx, _root) = test_app();
        app.account_generation = 3;
        app.playback_generation = 5;
        for (account, playback) in [(2, 5), (3, 4)] {
            deliver(
                &app,
                Kind::Resolve {
                    account_generation: account,
                    playback_generation: playback,
                    queued: false,
                    media: Box::default(),
                    audio: true,
                    preview: false,
                    download: false,
                    resume: false,
                },
                Ok(json!({"url":"https://media.test/old"})),
                None,
            );
        }
        app.poll(&ctx);
        assert!(app.effects.is_empty());
        deliver(
            &app,
            Kind::Resolve {
                account_generation: 3,
                playback_generation: 5,
                queued: false,
                media: Box::default(),
                audio: true,
                preview: false,
                download: false,
                resume: false,
            },
            Ok(json!({"url":"https://media.test/current"})),
            None,
        );
        app.poll(&ctx);
        assert!(matches!(app.effects.as_slice(), [Effect::Play(_)]));
    }
    #[test]
    fn feed_retry_respects_server_cooldown_preserves_content_and_is_bounded() {
        let (mut app, ctx, _root) = test_app();
        app.feeds[1].page.items.push(Media {
            title: "Keep me".into(),
            ..Default::default()
        });
        let before = std::time::Instant::now();
        for attempt in 1..=4 {
            app.feeds[1].retry_at = None;
            deliver(
                &app,
                Kind::Feed {
                    tab: 1,
                    generation: 0,
                    append: true,
                },
                Err("429".into()),
                Some(std::time::Duration::from_secs(60)),
            );
            app.poll(&ctx);
            assert_eq!(app.feeds[1].page.items[0].title, "Keep me");
            assert_eq!(app.feeds[1].failures, attempt.min(3));
            if attempt <= 3 {
                assert!(
                    app.feeds[1].retry_at.unwrap().duration_since(before)
                        >= std::time::Duration::from_secs(60)
                );
            } else {
                assert!(app.feeds[1].retry_at.is_none());
            }
        }
        app.feeds[1].generation = 1;
        deliver(
            &app,
            Kind::Feed {
                tab: 1,
                generation: 0,
                append: false,
            },
            Ok(serde_json::to_value(Page::default()).unwrap()),
            None,
        );
        app.poll(&ctx);
        assert_eq!(app.feeds[1].page.items.len(), 1);
    }
    #[test]
    fn android_back_closes_overlays_before_backgrounding() {
        let (mut app, ctx, _root) = test_app();
        app.settings_open = true;
        app.player_open = true;
        app.platform_status(&ctx, json!({"events":[{"kind":"back"}]}));
        assert!(!app.settings_open);
        assert!(app.player_open);
        assert!(app.effects.is_empty());
        app.platform_status(&ctx, json!({"events":[{"kind":"back"}]}));
        assert!(!app.player_open);
        assert!(app.effects.is_empty());
        app.platform_status(&ctx, json!({"events":[{"kind":"back"}]}));
        assert!(matches!(app.effects.as_slice(),[Effect::Command(v)] if v["op"]=="background"));
    }
    #[test]
    fn every_music_style_renders_and_pause_reduced_motion_and_cancel_preserve_state() {
        let (mut app, ctx, _root) = test_app();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(412.0, 915.0),
            )),
            ..Default::default()
        };
        app.tab = 2;
        app.player_open = true;
        app.player = json!({"title":"Synthetic song","playing":true,"duration":180,"position":60,"left":0.4,"right":0.7,"peak":0.8});
        for &style in ytui_core::MUSIC_PLAYER_STYLES {
            app.config.music_player_style = style.into();
            app.last_frame = std::time::Instant::now() - std::time::Duration::from_millis(50);
            let previous = app.phase;
            let output = ctx.run(input(), |ctx| app.frame(ctx));
            assert!(!output.shapes.is_empty());
            if style == "classic" {
                assert_eq!(previous, app.phase);
            } else {
                assert!(app.phase > previous);
            }
        }
        app.player["playing"] = json!(false);
        let frozen = app.phase;
        let _ = ctx.run(input(), |ctx| app.frame(ctx));
        assert_eq!(app.phase, frozen);
        app.player["playing"] = json!(true);
        app.config.reduced_motion = true;
        let _ = ctx.run(input(), |ctx| app.frame(ctx));
        assert_eq!(app.phase, frozen);
        app.player_open = false;
        app.settings_open = true;
        let original = app.config.download.audio;
        let _ = ctx.run(input(), |ctx| app.frame(ctx));
        app.config.download.audio = !original;
        app.platform_status(&ctx, json!({"events":[{"kind":"back"}]}));
        assert_eq!(app.config.download.audio, original);
        assert!(app.effects.is_empty());
        assert_eq!(
            lyric_text(&json!([{ "title":"Song", "artist":"Artist", "lines":[{"text":"Hello"}]}])),
            "Song — Artist\n\nHello"
        );
    }
    #[test]
    fn phone_tablet_and_all_theme_layouts_render_without_network() {
        let (mut app, ctx, _root) = test_app();
        for name in ytui_core::theme::THEMES {
            app.palette = ytui_core::theme::builtin(name).unwrap();
            for size in [
                Vec2::new(360.0, 800.0),
                Vec2::new(412.0, 915.0),
                Vec2::new(1280.0, 800.0),
            ] {
                for tab in 0..4 {
                    app.tab = tab;
                    let out = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| app.frame(ctx),
                    );
                    assert!(!out.shapes.is_empty());
                }
            }
        }
        for open in 0..3 {
            app.settings_open = open == 0;
            app.menu_open = open == 1;
            app.player_open = open == 2;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(360.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.frame(ctx),
            );
        }
        assert!(app.effects.is_empty());
    }
    #[test]
    fn phone_navigation_responds_to_touch_sized_pointer_input() {
        let (mut app, ctx, _root) = test_app();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 800.0));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| app.frame(ctx),
        );
        for pressed in [true, false] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events: vec![
                        egui::Event::PointerMoved(egui::pos2(250.0, 770.0)),
                        egui::Event::PointerButton {
                            pos: egui::pos2(250.0, 770.0),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| app.frame(ctx),
            );
        }
        assert_eq!(app.tab, 2);
    }
}
