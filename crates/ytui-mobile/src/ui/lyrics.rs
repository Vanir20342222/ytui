//! Rust lyric editor and virtualized timing view; remote lookup is always explicit.
use super::*;
use ytui_providers::lyrics::Track;

#[derive(Clone, Copy)]
pub(super) enum ReplyKind {
    Stored,
    Lookup,
    Saved,
    Removed,
}
pub(super) struct View {
    id: uuid::Uuid,
    media: Media,
    title: String,
    artist: String,
    text: String,
    offset_ms: i64,
    candidates: Vec<Track>,
    selected: usize,
    editing: bool,
    follow: bool,
    last_active: Option<usize>,
    pending: Option<u64>,
    error: String,
}
impl View {
    fn new(media: Media) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            title: media.title.clone(),
            artist: media.creator.clone(),
            media,
            text: String::new(),
            offset_ms: 0,
            candidates: Vec::new(),
            selected: 0,
            editing: false,
            follow: true,
            last_active: None,
            pending: None,
            error: String::new(),
        }
    }
    fn accept_text(&mut self, text: String) {
        let document = crate::lyrics::Document {
            title: self.title.clone(),
            artist: self.artist.clone(),
            text,
            offset_ms: self.offset_ms,
        };
        match document.track() {
            Ok(track) => {
                self.text = document.text;
                self.candidates = vec![track];
                self.selected = 0;
                self.last_active = None;
                self.error.clear();
            }
            Err(e) => self.error = e.to_string(),
        }
    }
    fn active(&self, player: &Value) -> Option<usize> {
        if player["media"]["url"].as_str() != Some(self.media.url.as_str()) {
            return None;
        }
        let position = player["position"].as_f64()? - self.offset_ms as f64 / 1000.0;
        self.candidates.get(self.selected)?.active_line(position)
    }
}
impl MobileApp {
    pub(super) fn open_lyrics(&mut self, ctx: &egui::Context) {
        let Ok(media) = serde_json::from_value::<Media>(self.player["media"].clone()) else {
            return;
        };
        self.close_lyrics();
        let url = media.url.clone();
        self.lyrics = Some(View::new(media));
        self.lyric_request(ctx, json!({"op":"lyrics_get","url":url}), ReplyKind::Stored);
    }
    fn lyric_request(&mut self, ctx: &egui::Context, request: Value, kind: ReplyKind) {
        let Some(view) = self.lyrics.as_mut() else {
            return;
        };
        if let Some(previous) = view.pending.take() {
            self.engine.cancel(previous);
            if let Some(task) = self.tasks.remove(&previous) {
                task.abort();
            }
        }
        let id = view.id;
        let pending = self.request(ctx, request, Kind::Lyrics(id, kind));
        self.lyrics.as_mut().unwrap().pending = Some(pending);
    }
    pub(super) fn close_lyrics(&mut self) {
        if let Some(view) = self.lyrics.take()
            && let Some(id) = view.pending
        {
            self.engine.cancel(id);
            if let Some(task) = self.tasks.remove(&id) {
                task.abort();
            }
        }
        self.lyric_import = None;
    }
    pub(super) fn import_lyrics(&mut self, text: &str) {
        if let Some(view) = self.lyrics.as_mut()
            && self.lyric_import.take() == Some(view.id)
        {
            view.accept_text(text.into());
        }
    }
    pub(super) fn lyric_reply(
        &mut self,
        id: uuid::Uuid,
        request: u64,
        kind: ReplyKind,
        result: Result<Value, String>,
    ) {
        let Some(view) = self
            .lyrics
            .as_mut()
            .filter(|v| v.id == id && v.pending == Some(request))
        else {
            return;
        };
        view.pending = None;
        match result {
            Err(e) => view.error = e,
            Ok(value) => {
                view.error.clear();
                match kind {
                    ReplyKind::Lookup => match serde_json::from_value::<Vec<Track>>(value) {
                        Ok(tracks) => {
                            view.candidates = tracks;
                            view.selected = 0;
                            view.last_active = None;
                            view.offset_ms = 0;
                            view.text = view.candidates.first().map(lrc_text).unwrap_or_default();
                        }
                        Err(_) => view.error = "Invalid lyric response".into(),
                    },
                    ReplyKind::Stored | ReplyKind::Saved => {
                        if value.is_null() {
                            return;
                        }
                        match serde_json::from_value::<crate::lyrics::Document>(
                            value["document"].clone(),
                        ) {
                            Ok(document) => {
                                view.title = document.title;
                                view.artist = document.artist;
                                view.offset_ms = document.offset_ms;
                                view.accept_text(document.text);
                            }
                            Err(_) => view.error = "Invalid saved lyric document".into(),
                        }
                    }
                    ReplyKind::Removed => {
                        view.candidates.clear();
                        view.text.clear();
                        view.offset_ms = 0;
                        view.last_active = None;
                    }
                }
            }
        }
    }
    pub(super) fn lyrics_window(&mut self, ctx: &egui::Context) {
        let safe = self.safe_rect(ctx);
        let Some(view) = self.lyrics.as_mut() else {
            return;
        };
        let mut open = true;
        let mut action = None;
        let mut import = false;
        let mut seek = None;
        egui::Window::new("Lyrics").id(egui::Id::new("lyrics"))
            .open(&mut open).constrain_to(safe).default_width((safe.width()-24.0).min(640.0))
            .vscroll(true).show(ctx,|ui|{
                if view.pending.is_some(){ui.disable();}
                ui.label("Song title"); ui.text_edit_singleline(&mut view.title);
                ui.label("Artist"); ui.text_edit_singleline(&mut view.artist);
                ui.small("Only title and artist are sent to LRCLIB when you tap Find. No cookies are sent.");
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(view.pending.is_none(),egui::Button::new("Find on LRCLIB")).clicked(){action=Some((json!({"op":"lyrics","media":view.media,"title":view.title,"artist":view.artist}),ReplyKind::Lookup));}
                    if ui.add_enabled(view.pending.is_none(),egui::Button::new("Import LRC / text")).clicked(){import=true;}
                    ui.toggle_value(&mut view.editing,"Edit text");
                });
                if view.pending.is_some(){ui.spinner();}
                if !view.error.is_empty(){ui.colored_label(Color32::from_rgb(230,120,100),&view.error);}
                if view.candidates.len()>1 {
                    let previous=view.selected;
                    egui::ComboBox::from_label("Version").selected_text(format!("{} of {}",view.selected+1,view.candidates.len())).show_ui(ui,|ui|{
                        for (i,track) in view.candidates.iter().enumerate(){ui.selectable_value(&mut view.selected,i,format!("{} · {} · {}",track.artist,track.album,timestamp(track.duration)));}
                    });
                    if previous!=view.selected{view.text=lrc_text(&view.candidates[view.selected]);view.last_active=None;}
                }
                ui.horizontal_wrapped(|ui|{
                    ui.checkbox(&mut view.follow,"Follow playback");
                    ui.label("Delay (ms)");
                    if ui.add(egui::DragValue::new(&mut view.offset_ms).range(-120_000..=120_000).speed(50)).changed(){view.last_active=None;}
                });
                ui.small("Positive delay shows each lyric later. Tap a timed line to seek.");
                if view.editing {
                    ui.add(egui::TextEdit::multiline(&mut view.text).desired_rows(8).char_limit(512*1024).desired_width(f32::INFINITY));
                    if ui.button("Apply edited text").clicked(){view.accept_text(view.text.clone());}
                }
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(view.pending.is_none()&&!view.text.trim().is_empty(),egui::Button::new("Save locally")).clicked(){action=Some((json!({"op":"lyrics_save","url":view.media.url,"document":{"title":view.title,"artist":view.artist,"text":view.text,"offset_ms":view.offset_ms}}),ReplyKind::Saved));}
                    if ui.add_enabled(view.pending.is_none(),egui::Button::new("Remove saved lyrics")).clicked(){action=Some((json!({"op":"lyrics_remove","url":view.media.url}),ReplyKind::Removed));}
                });
                let matches=self.player["media"]["url"].as_str()==Some(view.media.url.as_str());
                if !matches{ui.label("Timing paused: a different song is playing.");}
                let active=view.active(&self.player);
                let follow=view.follow && active!=view.last_active;
                if let Some(track)=view.candidates.get(view.selected) {
                    ui.label(format!("{} — {}",track.title,track.artist));
                    if track.instrumental{ui.label("Instrumental — no sung lyrics");}
                    let row=48.0;
                    let mut scroll=egui::ScrollArea::vertical().id_salt(("lyric-lines",view.id)).max_height((safe.height()*0.4).max(100.0));
                    if follow && let Some(index)=active {scroll=scroll.vertical_scroll_offset((index as f32*(row+ui.spacing().item_spacing.y)-78.0).max(0.0));}
                    scroll.show_rows(ui,row,track.lines.len(),|ui,range|{
                        for i in range {
                            let line=&track.lines[i];
                            let timed=matches && line.at_ms.is_some();
                            if ui.add_sized([ui.available_width(),48.0],egui::Button::new(RichText::new(&line.text).size(18.0)).truncate().selected(active==Some(i))).clicked() && timed {
                                seek=line.at_ms.map(|ms|(ms as f64+view.offset_ms as f64).max(0.0)/1000.0);
                            }
                        }
                    });
                } else if view.pending.is_none(){ui.label("Find a version or import a local LRC/text file.");}
                view.last_active=active;
            });
        if import {
            self.lyric_import = Some(view.id);
            self.effects.push(Effect::Pick("lyrics".into()));
        }
        if let Some(seconds) = seek {
            self.effects
                .push(Effect::Command(json!({"op":"seek","seconds":seconds})));
        }
        if let Some((request, kind)) = action {
            self.lyric_request(ctx, request, kind);
        }
        if !open {
            self.close_lyrics();
        }
    }
}
fn lrc_text(track: &Track) -> String {
    track
        .lines
        .iter()
        .map(|line| match line.at_ms {
            Some(at) => format!(
                "[{:02}:{:02}.{:03}]{}",
                at / 60000,
                at / 1000 % 60,
                at % 1000,
                line.text
            ),
            None => line.text.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lyric_editor_and_synced_view_render_on_phone_and_tablet_in_all_themes() {
        let (mut app, ctx, _root) = super::super::tests::test_app();
        let media = Media {
            url: "https://media.test/song".into(),
            title: "Grüße 🎵".into(),
            creator: "Artist".into(),
            ..Default::default()
        };
        app.player = json!({"media":media,"position":3.0,"playing":true});
        let mut view = View::new(media);
        view.accept_text("[00:01.234]First\n[00:02.345]Second\n[00:04.567]Third".into());
        app.lyrics = Some(view);
        for name in ytui_core::theme::THEMES {
            app.palette = ytui_core::theme::builtin(name).unwrap();
            for size in [Vec2::new(360.0, 800.0), Vec2::new(1280.0, 800.0)] {
                for edit in [false, true] {
                    app.lyrics.as_mut().unwrap().editing = edit;
                    let out = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| app.frame(ctx),
                    );
                    assert!(!out.shapes.is_empty());
                    assert_eq!(app.lyrics.as_ref().unwrap().last_active, Some(1));
                }
            }
        }
        assert!(app.effects.is_empty());
    }
    #[test]
    fn timing_offsets_and_timestamp_roundtrip_preserve_track_identity() {
        let media = Media {
            url: "https://media.test/first".into(),
            ..Default::default()
        };
        let mut view = View::new(media.clone());
        view.accept_text("[00:01.234]Grüße 🎵\n[00:03.987]Second".into());
        view.offset_ms = 500;
        let mut player = json!({"media":media,"position":1.7});
        assert_eq!(view.active(&player), None);
        player["position"] = json!(1.735);
        assert_eq!(view.active(&player), Some(0));
        player["media"]["url"] = json!("https://media.test/second");
        assert_eq!(view.active(&player), None);
        let track = &view.candidates[0];
        assert_eq!(
            ytui_providers::lyrics::parse(&lrc_text(track)).unwrap(),
            track.lines
        );
    }
    #[test]
    fn dismissed_or_superseded_lyrics_cannot_be_reopened_by_late_requests_or_imports() {
        let (mut app, ctx, _root) = super::super::tests::test_app();
        app.player = json!({"media":{"url":"https://media.test/first","title":"First"}});
        app.open_lyrics(&ctx);
        let old = app.lyrics.as_ref().unwrap().id;
        let pending = app.lyrics.as_ref().unwrap().pending.unwrap();
        app.lyric_import = Some(old);
        app.close_lyrics();
        app.lyric_reply(old, pending, ReplyKind::Lookup, Ok(json!([])));
        app.import_lyrics("[00:01]Late");
        assert!(app.lyrics.is_none());
        app.open_lyrics(&ctx);
        let current = app.lyrics.as_ref().unwrap().id;
        assert_ne!(old, current);
        app.lyric_import = Some(old);
        app.import_lyrics("Wrong song");
        assert!(app.lyrics.as_ref().unwrap().text.is_empty());
        app.lyric_reply(old, pending, ReplyKind::Lookup, Err("Late failure".into()));
        assert!(app.lyrics.as_ref().unwrap().error.is_empty());
        app.platform_status(&ctx, json!({"events":[{"kind":"back"}]}));
        assert!(app.lyrics.is_none());
        assert!(app.effects.is_empty());
    }
}
