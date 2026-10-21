use crate::{
    editor::Editor,
    forms::{Form, FormKind},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
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
}
pub enum Modal {
    Help,
    Themes(usize),
    Form(Box<Form>),
    Queue(usize),
}
pub struct Input {
    pub command: bool,
    pub editor: Editor,
}
pub enum UiEvent {
    Page {
        tab: usize,
        generation: u64,
        append: bool,
        result: Result<Page>,
    },
    Image {
        generation: u64,
        result: Result<Protocol>,
    },
    Config {
        config: Box<Config>,
        account: Option<Session>,
        result: Result<()>,
    },
    Auth(Result<Session>),
    Disconnected(Result<()>),
    Status(String),
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
    pub demo: bool,
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
    pub image: Option<Protocol>,
    pub image_area: Rect,
    image_key: Option<(String, u16, u16)>,
    image_generation: u64,
    image_cancel: CancellationToken,
    image_due: Instant,
    pub image_error: Option<String>,
    images: ImageCache,
}
impl App {
    pub fn new(
        config: Config,
        paths: Paths,
        snapshot: Snapshot,
        session: Option<Arc<Session>>,
        handles: RuntimeHandles,
        demo: bool,
    ) -> Result<Self> {
        let RuntimeHandles {
            player,
            jobs: job_tx,
            picker,
        } = handles;
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
            config,
            paths,
            snapshot,
            tab: 1,
            tabs: std::array::from_fn(|_| TabView::default()),
            download_selected: 0,
            modal: None,
            input: None,
            status: if session.is_some() {
                "Connected · :home :subscriptions :library".into()
            } else {
                "Anonymous · :auth connects your browser · ? for help".into()
            },
            palette: theme::builtin("ytui-dark")?,
            player_status: PlayerStatus::default(),
            demo,
            quit: false,
            launch_cliamp: false,
            dirty: false,
            redraw: true,
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
            image: None,
            image_area: Rect::default(),
            image_key: None,
            image_generation: 0,
            image_cancel: CancellationToken::new(),
            image_due: Instant::now(),
            image_error: None,
            images,
        };
        if demo {
            app.load_demo();
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
            self.tabs[tab].page=Page{title:if tab==3{"cliamp radio"}else if tab==2{"Your music"}else{"For you"}.into(),items:(0..18).map(|i|Media{id:format!("demo-{i}"),title:["A quieter corner of the internet","Late night, soft keys","Building something that lasts","A walk through the northern lights","The art of paying attention","Small discoveries, every day"][i%6].into(),creator:if tab==2{"Studio Sessions"}else if tab==3{"Independent radio"}else{"Everyday Curiosity"}.into(),duration:Some((i as u64+1)*97),description:"Fictional demo content. Press / to explore the interface, t to choose a theme, or ? for shortcuts. No account or network is used for this catalog.".into(),kind:if tab==2{MediaKind::Song}else if tab==3{MediaKind::Station}else{MediaKind::Video},live:tab==3,..Default::default()}).collect(),..Default::default()};
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
        self.status = "DEMO · Fictional catalog · No account data".into();
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
        if self.demo {
            self.status = "Demo catalog · Network actions are disabled".into();
            return;
        }
        let view = &mut self.tabs[tab];
        if !append {
            if let Some(old) = view.request.take()
                && !view.page.items.is_empty()
            {
                view.history.push((old, view.page.clone(), view.selected));
                if view.history.len() > 30 {
                    view.history.remove(0);
                }
            }
            view.request = Some(request.clone());
        }
        view.cancel.cancel();
        view.cancel = CancellationToken::new();
        let cancel = view.cancel.clone();
        view.generation += 1;
        let generation = view.generation;
        view.loading = true;
        view.error = None;
        let browser = self.browser.clone();
        let radio = self.radio.clone();
        let favorites = self.snapshot.favorites.clone();
        let custom = self.config.custom_stations.clone();
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
                if tab==3 {
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
                        if append {
                            for media in page.items {
                                if !view.page.items.iter().any(|m| m.id == media.id) {
                                    view.page.items.push(media);
                                }
                            }
                            view.page.continuation = page.continuation;
                        } else {
                            view.page = page;
                            view.selected = 0;
                        }
                        if tab == self.tab {
                            self.selection_changed();
                        }
                    }
                    Err(e) => view.error = Some(ytui_services::process::diagnostic(&e.to_string())),
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
                    Err(_) => {
                        self.image = None;
                        self.image_error = Some("Thumbnail unavailable".into());
                    }
                }
            }
            UiEvent::Config {
                config,
                account,
                result,
            } => match result {
                Err(e) => self.status = format!("Could not save settings: {e}"),
                Ok(()) => {
                    let account_changed = self.config.account_index != config.account_index;
                    self.config = *config;
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
                            self.images = ImageCache::new(client, self.paths.cache.join("images"));
                        }
                        Err(e) => self.status = e.to_string(),
                    }
                    self.status =
                        "Settings saved · changes apply to new downloads and playback".into();
                    self.selection_changed();
                    if account_changed {
                        self.reset_account();
                    }
                }
            },
            UiEvent::Auth(result) => match result {
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
            },
            UiEvent::Disconnected(result) => match result {
                Ok(()) => {
                    self.session = None;
                    self.reset_account();
                    self.status = "Disconnected · browser cookies were left intact".into();
                }
                Err(e) => self.status = e.to_string(),
            },
            UiEvent::Status(message) => {
                if !message.is_empty() {
                    self.status = message;
                }
            }
        }
        self.redraw = true;
    }
    fn reset_account(&mut self) {
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
        }
        if let Ok(client) = ytui_services::http_client(self.config.proxy.as_deref()) {
            self.browser = Browser::new(client, self.session.clone(), self.config.account_index);
        }
        let _ = self
            .player
            .try_send(PlayerCommand::Session(self.session.clone()));
        self.snapshot.playback = PlaybackQueue::default();
        self.dirty = true;
        self.selection_changed();
        if self.tab == 1 || self.tab == 2 {
            self.navigate(self.tab, BrowseRequest::Home, false);
        }
    }
    pub fn save_config(&mut self, c: Config) {
        if self.demo {
            self.config = c;
            if let Ok(p) = theme::builtin(&self.config.theme) {
                self.palette = p;
            }
            self.status = "Demo theme/settings applied temporarily".into();
            self.redraw = true;
            return;
        }
        let paths = self.paths.clone();
        let account_changed = self.config.account_index != c.account_index;
        let current = self.session.clone();
        self.tasks.spawn(async move {
            let mut account = None;
            let result = async {
                if account_changed && let Some(session) = current {
                    let mut candidate = (*session).clone();
                    let browser = Browser::new(
                        ytui_services::http_client(c.proxy.as_deref())?,
                        Some(session),
                        c.account_index,
                    );
                    candidate.account = browser.validate().await?;
                    let store = SessionStore::new(&paths, &c.auth_store);
                    let saved = candidate.clone();
                    tokio::task::spawn_blocking(move || store.save(&saved)).await??;
                    account = Some(candidate);
                }
                let saved = c.clone();
                tokio::task::spawn_blocking(move || paths.save_config(&saved)).await??;
                Ok(())
            }
            .await;
            UiEvent::Config {
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
                    if input.command {
                        if let Err(e) = self.command(&text) {
                            self.status = e.to_string();
                        }
                    } else {
                        self.search(&text);
                    }
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
            Modal::Help => {
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
                    if let Some(m) = self.snapshot.playback.items.get(*index).cloned() {
                        self.snapshot.playback.current = Some(*index);
                        self.play_existing(m);
                        self.dirty = true;
                    }
                }
                _ => self.modal = Some(modal),
            },
            Modal::Form(form) => {
                match key.code {
                    KeyCode::Up | KeyCode::BackTab => {
                        form.selected = form.selected.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        form.selected = (form.selected + 1).min(form.fields.len() - 1)
                    }
                    KeyCode::Char(' ') if form.fields[form.selected].toggle => form.toggle(),
                    KeyCode::Enter => {
                        let outcome = match &form.kind {
                            FormKind::Download(_, _) => form.download_values().map(|(m, o)| {
                                self.enqueue_download(m, o);
                            }),
                            FormKind::Settings(_) => {
                                form.config_values().map(|c| self.save_config(c))
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
        } else if let Some(Modal::Form(form)) = &mut self.modal {
            if !form.fields[form.selected].toggle {
                form.fields[form.selected].value.insert(text);
            }
        } else {
            let mut editor = Editor::default();
            editor.insert(text);
            self.input = Some(Input {
                command: false,
                editor,
            });
        }
        self.redraw = true;
    }
    pub fn change_tab(&mut self, tab: usize) {
        self.tab = tab % 4;
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
        self.selection_changed();
        self.redraw = true;
    }
    fn selection_changed(&mut self) {
        self.image_cancel.cancel();
        self.image_cancel = CancellationToken::new();
        self.image = None;
        self.image_key = None;
        self.image_error = None;
        self.image_generation += 1;
        self.image_due = Instant::now() + Duration::from_millis(120);
    }
    pub fn refresh_image(&mut self) {
        if Instant::now() < self.image_due
            || self.config.image_protocol == "none"
            || !self.config.thumbnails
            || self.modal.is_some()
            || self.image_area.width == 0
            || self.image_area.height == 0
        {
            return;
        }
        let Some(media) = self.selected() else {
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
        self.image_key = Some(key);
        self.image_generation += 1;
        let generation = self.image_generation;
        let cache = self.images.clone();
        let picker = self.picker.clone();
        let area = Rect::new(0, 0, self.image_area.width, self.image_area.height);
        let cancel = self.image_cancel.clone();
        self.tasks.spawn(async move {
            let work = async {
                let image = cache.load(&media.thumbnail).await?;
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
    pub fn action(&mut self, action: Action) {
        match action {
            Action::Fallback => {
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
                } else if let Some(media) = self.selected()
                    && !media.collection()
                {
                    let audio = self.tab == 2
                        || media.kind == MediaKind::Song
                        || media.kind == MediaKind::Station;
                    self.snapshot.playback.play(media.clone());
                    self.dirty = true;
                    let _ = self
                        .player
                        .try_send(PlayerCommand::PlayFallback { media, audio });
                    self.status = "Playing with explicit yt-dlp fallback".into();
                }
            }
            Action::Quit => self.quit = true,
            Action::Help => self.modal = Some(Modal::Help),
            Action::Search => {
                self.input = Some(Input {
                    command: false,
                    editor: Editor::default(),
                })
            }
            Action::Command => {
                self.input = Some(Input {
                    command: true,
                    editor: Editor::default(),
                })
            }
            Action::NextTab => self.change_tab((self.tab + 1) % 4),
            Action::PreviousTab => self.change_tab((self.tab + 3) % 4),
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Open => {
                if self.tab == 0 {
                    if let Some(job) = self.snapshot.downloads.jobs.get(self.download_selected)
                        && job.state == JobState::Done
                        && let Some(path) = job.outputs.first()
                    {
                        let mut media = job.media.clone();
                        media.url = path.to_string_lossy().into();
                        self.play(media);
                    }
                } else if let Some(media) = self.selected() {
                    if let Some(id) = media.browse_id {
                        self.navigate(self.tab, BrowseRequest::Collection(id), false);
                    } else if let Some(url) = media.tracks_url {
                        self.navigate(3, BrowseRequest::CliampTracks(url), false);
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
                {
                    let _ = self.player.try_send(PlayerCommand::Preview(media));
                }
            }
            Action::Download => {
                if let Some(media) = self.selected() {
                    let mut o = self.config.download.clone();
                    o.audio = self.tab == 2;
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
                if self.player_status.preview {
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
                    self.selection_changed();
                }
            }
            Action::Refresh => {
                if self.tab > 0
                    && let Some(req) = self.tabs[self.tab].request.clone()
                {
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
                if let Some(m) = self.selected()
                    && !m.collection()
                {
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
                if let Some(m) = self.snapshot.playback.advance(false) {
                    self.play_existing(m);
                    self.dirty = true;
                }
            }
            Action::PreviousTrack => {
                if let Some(m) = self.snapshot.playback.previous() {
                    self.play_existing(m);
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
            Action::Stop => {
                let _ = self.player.try_send(PlayerCommand::Stop);
                self.snapshot.playback.current = None;
                self.dirty = true;
            }
            Action::Cliamp => self.launch_cliamp = true,
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
    pub fn enqueue_download(&mut self, mut media: Media, options: DownloadOptions) {
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
                    && j.options.audio == options.audio
                    && j.options.quality == options.quality
                    && j.state != JobState::Error
                    && j.state != JobState::Cancelled
            })
        {
            self.status = "This item is already in your download queue/history".into();
            return;
        }
        self.snapshot.downloads.jobs.push(Job::new(media, options));
        self.dirty = true;
        self.status = "Added to Downloads · 1 opens the queue".into();
    }
    pub fn search(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        if let Ok(url) = normalize_media_url(query) {
            let media = Media {
                title: query.into(),
                url,
                ..Default::default()
            };
            let o = self.config.download.clone();
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
    pub fn play(&mut self, media: Media) {
        if self.demo {
            self.status = "Playback is disabled for fictional demo content".into();
            return;
        }
        self.snapshot.playback.play(media.clone());
        self.dirty = true;
        self.play_existing(media);
    }
    pub fn play_existing(&mut self, media: Media) {
        let audio =
            media.kind == MediaKind::Song || media.kind == MediaKind::Station || self.tab == 2;
        let _ = self.player.try_send(PlayerCommand::Play { media, audio });
    }
    pub async fn tick(&mut self) {
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
            }
        }
        if !self.demo {
            for job in &mut self.snapshot.downloads.jobs {
                if self.running.len() >= self.config.concurrency {
                    break;
                }
                if job.state == JobState::Queued && !self.running.contains_key(&job.id) {
                    job.state = JobState::Resolving;
                    let cancel = CancellationToken::new();
                    let task = tokio::spawn(download::execute(
                        job.clone(),
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
        if self.theme_rx.has_changed().unwrap_or(false) {
            self.palette = self.theme_rx.borrow_and_update().clone();
            self.redraw = true;
        }
        self.refresh_image();
    }
    pub async fn shutdown(&mut self) {
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
        self.tasks.shutdown().await;
        for job in &mut self.snapshot.downloads.jobs {
            if job.state.active() {
                job.invalidate(JobState::Queued);
            }
        }
    }
    fn import_auth(&mut self, browser: String, profile: String, file: String) {
        if self.demo {
            self.status = "Account access is disabled in demo mode".into();
            return;
        }
        self.status = "Reading selected profile and validating session…".into();
        let store = SessionStore::new(&self.paths, &self.config.auth_store);
        let config = self.config.clone();
        self.tasks.spawn(async move {
            let result = async {
                let candidate = tokio::task::spawn_blocking(move || {
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
                .await??;
                let api = Browser::new(
                    ytui_services::http_client(config.proxy.as_deref())?,
                    Some(Arc::new(candidate.clone())),
                    config.account_index,
                );
                let mut candidate = candidate;
                candidate.account = api.validate().await?;
                let saved = candidate.clone();
                tokio::task::spawn_blocking(move || store.save(&saved)).await??;
                Ok(candidate)
            }
            .await;
            UiEvent::Auth(result)
        });
    }
    pub fn command(&mut self, input: &str) -> Result<()> {
        let parts = shell_words::split(input)?;
        let Some(cmd) = parts.first().map(String::as_str) else {
            return Ok(());
        };
        let value = parts.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        let browse_tab = if self.tab == 0 { 1 } else { self.tab };
        match cmd.trim_start_matches('/') {
            "quit" | "q" => self.quit = true,
            "help" => self.modal = Some(Modal::Help),
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
            "songs" | "albums" | "artists" | "videos" => {
                self.change_tab(2);
                if value.is_empty() {
                    let id = match cmd {
                        "albums" => "FEmusic_liked_albums",
                        "artists" => "FEmusic_library_corpus_artists",
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
                    self.modal = Some(Modal::Form(Box::new(Form::auth())));
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
                let store = SessionStore::new(&self.paths, &self.config.auth_store);
                self.tasks.spawn(async move {
                    UiEvent::Disconnected(
                        tokio::task::spawn_blocking(move || store.disconnect())
                            .await
                            .map_err(anyhow::Error::from)
                            .and_then(|r| r),
                    )
                });
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
            "export" => {
                ensure!(!value.is_empty(), "Provide an export path");
                let data = serde_json::to_vec_pretty(&self.snapshot)?;
                ytui_services::paths::atomic_write(&PathBuf::from(value), &data)?;
                self.status = "Queue/history exported".into();
            }
            "open" => {
                let media = self.selected().context("Select an item")?;
                ytui_core::http_url(&media.url)?;
                self.tasks.spawn(async move {
                    let result = ytui_services::process::run(
                        ytui_services::process::ProcessSpec::new("xdg-open", vec![media.url], 10),
                        CancellationToken::new(),
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
