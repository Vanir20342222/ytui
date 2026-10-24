use crate::app::{App, Modal, TAB_NAMES};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table,
        TableState, Wrap,
    },
};
use ratatui_image::Image;
use std::borrow::Cow;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use ytui_core::{JobState, shortcuts::REGISTRY};

pub fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}
fn themed(app: &App, c: (u8, u8, u8)) -> Color {
    if app.palette.name != "terminal" {
        return rgb(c);
    }
    let p = &app.palette;
    if c == p.background {
        Color::Reset
    } else if c == p.surface || c == p.selection {
        Color::DarkGray
    } else if c == p.text {
        Color::White
    } else if c == p.accent {
        Color::Cyan
    } else if c == p.error {
        Color::Red
    } else if c == p.success {
        Color::Green
    } else if c == p.warning {
        Color::Yellow
    } else {
        Color::Gray
    }
}
fn translated<'a>(app: &App, text: &'a str) -> Cow<'a, str> {
    let core = text.trim();
    let translation = app.tr(core);
    if translation == core {
        return Cow::Borrowed(text);
    }
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    Cow::Owned(format!("{}{}{}", &text[..start], translation, &text[end..]))
}
fn panel(title: impl Into<Line<'static>>, app: &App) -> Block<'static> {
    let mut title = title.into();
    for span in &mut title.spans {
        if let Cow::Owned(translation) = translated(app, &span.content) {
            span.content = translation.into();
        }
    }
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(themed(app, app.palette.muted)))
        .style(
            Style::default()
                .bg(themed(app, app.palette.background))
                .fg(themed(app, app.palette.text)),
        )
}
mod lyrics;
mod menus;
mod settings;
mod workspace;
pub use workspace::render;
fn time(seconds: f64) -> String {
    let n = seconds.max(0.0) as u64;
    format!("{}:{:02}", n / 60, n % 60)
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}
fn modal(f: &mut Frame, app: &mut App, area: Rect) {
    if let Some(Modal::Form(form)) = &app.modal
        && form.is_grouped()
    {
        let hits = settings::render(f, app, form, centered(area, 112, 34));
        app.hit_targets.extend(hits);
        return;
    }
    let Some(modal) = &app.modal else {
        return;
    };
    let size = match modal {
        Modal::PendingText { .. } => (90, 12),
        Modal::Lyrics(_) => (105, 32),
        Modal::Actions { .. } => (96, 32),
        Modal::Categories(_) => (70, 26),
        Modal::Feeds(_) => (64, app.feed_choices().len() as u16 * 3 + 4),
        Modal::Help(_) => (90, 35),
        Modal::Themes(_) => (52, 20),
        Modal::Form(form) => (86, form.fields.len() as u16 + 8),
        Modal::Queue(_) => (
            85,
            (app.snapshot.playback.items.len() as u16)
                .saturating_add(4)
                .clamp(6, 25),
        ),
        Modal::Resume(_) => (
            90,
            (app.resume_positions().len() as u16)
                .saturating_mul(2)
                .saturating_add(4)
                .clamp(7, 25),
        ),
        Modal::Watches(_) => (
            100,
            (app.snapshot.watches.len() as u16)
                .saturating_mul(2)
                .saturating_add(4)
                .clamp(7, 28),
        ),
        Modal::History(_) => (105, 32),
        Modal::Groups(_) => (85, 25),
        Modal::Destinations(_) => (
            95,
            (app.snapshot.recent_destinations.len() as u16)
                .saturating_add(4)
                .clamp(8, 22),
        ),
        Modal::Outputs { job, .. } => (
            105,
            app.snapshot
                .downloads
                .jobs
                .iter()
                .find(|download| download.id == *job)
                .map_or(8, |download| {
                    (download.outputs.len() as u16)
                        .saturating_mul(2)
                        .saturating_add(4)
                        .clamp(8, 28)
                }),
        ),
        Modal::Account { profiles, .. } => (
            100,
            (profiles.len() as u16)
                .saturating_mul(2)
                .saturating_add(12)
                .clamp(14, 28),
        ),
        Modal::PlayerSettings { section, .. } => (
            105,
            (app.playback_choices(*section).len() as u16)
                .saturating_add(7)
                .clamp(12, 30),
        ),
        Modal::Text { .. } => (90, 28),
    };
    let rect = centered(area, size.0, size.1);
    f.render_widget(Clear, rect);
    match modal {
        Modal::PendingText { title, .. } => {
            f.render_widget(
                Paragraph::new(app.tr("Loading… Esc closes this panel."))
                    .block(panel(format!(" {} ", app.tr(title)), app)),
                rect,
            );
        }
        Modal::Lyrics(view) => lyrics::render(f, app, view, rect),
        Modal::Actions { selected, query } => {
            let hits = menus::actions(f, app, *selected, query, rect);
            app.hit_targets.extend(hits);
        }
        Modal::Categories(selected) => {
            let hits = menus::categories(f, app, *selected, rect);
            app.hit_targets.extend(hits);
        }
        Modal::Feeds(selected) => {
            let block = panel(" Browse feeds & library ", app)
                .title_bottom(Line::from(" ↑/↓ choose · Enter opens · Esc closes ").centered())
                .border_style(Style::default().fg(themed(app, app.palette.accent)));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let items = app
                .feed_choices()
                .iter()
                .map(|(label, description, _)| {
                    ListItem::new(vec![
                        Line::styled(app.tr(label), Style::default().add_modifier(Modifier::BOLD)),
                        Line::styled(
                            app.tr(description),
                            Style::default().fg(themed(app, app.palette.muted)),
                        ),
                        Line::from(""),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(
                        Style::default()
                            .bg(themed(app, app.palette.selection))
                            .fg(themed(app, app.palette.accent)),
                    )
                    .highlight_symbol(" ▸ "),
                inner,
                &mut state,
            );
        }
        Modal::Text {
            title,
            lines,
            offset,
        } => {
            f.render_widget(
                Paragraph::new(
                    lines
                        .iter()
                        .map(|s| Line::from(s.as_str()))
                        .collect::<Vec<_>>(),
                )
                .scroll((*offset as u16, 0))
                .wrap(Wrap { trim: true })
                .block(panel(format!(" {title} · ↑/↓ scroll · Esc closes "), app)),
                rect,
            );
        }
        Modal::Help(offset) => {
            let mut lines = vec![
                Line::styled(
                    app.tr("Make yourself at home"),
                    Style::default()
                        .fg(themed(app, app.palette.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Line::from(app.tr("↑/↓ or PgUp/PgDn scroll · Esc closes · F2 → Keyboard bindings")),
                Line::from(""),
            ];
            for (_, action, _, description) in REGISTRY {
                let mapped = workspace::key(app, *action);
                lines.push(Line::from(format!(
                    "{mapped:>12}   {}",
                    app.tr(description)
                )));
            }
            lines.extend([
                Line::from(""),
                Line::from(app.tr("F2 Actions: account, downloads, library, radio, profiles, VPN and diagnostics.")),
                Line::from(app.tr(", opens General settings. F1 shows or hides the bottom shortcut hints.")),
                Line::from(app.tr("o opens player controls. G opens settings inside the mpv window.")),
                Line::from(app.tr("F3 opens server-provided feed categories; scroll to fetch more results.")),
            ]);
            f.render_widget(
                Paragraph::new(lines)
                    .scroll((*offset as u16, 0))
                    .block(panel(" Keyboard & navigation ", app))
                    .wrap(Wrap { trim: true }),
                rect,
            );
        }
        Modal::Themes(selected) => {
            let items = theme_items(app);
            let list = List::new(items)
                .block(panel(" Themes · Enter applies · Esc cancels ", app))
                .highlight_style(
                    Style::default()
                        .bg(themed(app, app.palette.selection))
                        .fg(themed(app, app.palette.accent)),
                )
                .highlight_symbol("▸ ");
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(list, rect, &mut state);
        }
        Modal::History(view) => {
            let rows = view.rows(&app.snapshot.downloads.jobs);
            let stats = ytui_core::history::Statistics::from_jobs(rows.iter().map(|(_, j)| *j));
            let block = panel(
                format!(
                    " Downloads · {}{} · {} matches ",
                    view.filter.name(),
                    if view.group.is_some() {
                        " · group"
                    } else {
                        ""
                    },
                    rows.len()
                ),
                app,
            );
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let chunks = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(inner);
            f.render_widget(
                Paragraph::new(format!(
                    " {} complete · {} failed · {} audio · {} output paths · {:.1}h completed media\n {:.1} MiB recorded transfer bytes · search: {}",
                    stats.completed,
                    stats.failed,
                    stats.audio,
                    stats.outputs,
                    stats.media_seconds as f64 / 3600.0,
                    stats.recorded_bytes as f64 / 1048576.0,
                    if view.query.is_empty() { "—" } else { &view.query },
                ))
                .style(Style::default().fg(themed(app, app.palette.muted))),
                chunks[0],
            );
            let items = rows
                .iter()
                .map(|(_, job)| {
                    ListItem::new(vec![
                        Line::from(format!(
                            "{}{:?} · {}{}",
                            job.entry_index
                                .map_or_else(String::new, |index| format!("{index}. ")),
                            job.state,
                            job.media.title,
                            if job.group_id.is_some() {
                                " · [group]"
                            } else {
                                ""
                            }
                        )),
                        Line::styled(
                            job.outputs
                                .last()
                                .map(|p| p.display().to_string())
                                .or_else(|| job.error.clone())
                                .unwrap_or_else(|| job.media.creator.clone()),
                            Style::default().fg(themed(app, app.palette.muted)),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default()
                .with_selected(Some(view.selected.min(rows.len().saturating_sub(1))));
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                chunks[1],
                &mut state,
            );
            f.render_widget(Paragraph::new(" ↑/↓ navigate · f filter · r retry · o files · Enter queue item · g group\n F2 → Downloads · Esc closes").style(Style::default().fg(themed(app, app.palette.accent))), chunks[2]);
        }
        Modal::Groups(selected) => {
            let groups = app.download_groups();
            let items = groups
                .iter()
                .map(|(id, title, count)| {
                    ListItem::new(vec![
                        Line::from(format!("{title} · {count} items")),
                        Line::styled(
                            id.clone(),
                            Style::default().fg(themed(app, app.palette.muted)),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(
                List::new(items)
                    .block(panel(" Download groups · Enter expands · Esc closes ", app))
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                rect,
                &mut state,
            );
        }
        Modal::Watches(selected) => {
            if app.snapshot.watches.is_empty() {
                f.render_widget(Paragraph::new("\n  No collection watches yet.\n  Open a playlist, then F2 → Watch this collection.")
                    .wrap(Wrap { trim: true }).block(panel(" Collection watches · Esc closes ", app)), rect);
                return;
            }
            let items = app
                .snapshot
                .watches
                .iter()
                .map(|w| {
                    ListItem::new(vec![
                        Line::from(format!(
                            "{} · {} · every {}m",
                            w.title,
                            if !w.enabled {
                                "Paused"
                            } else if w.seen.is_none() {
                                "Baseline pending"
                            } else {
                                "Watching"
                            },
                            w.interval_seconds / 60
                        )),
                        Line::styled(
                            w.error.clone().unwrap_or_else(|| w.id.to_string()),
                            Style::default().fg(themed(app, app.palette.muted)),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(
                List::new(items)
                    .block(panel(
                        " Collection watches · Space pause/resume · Del removes · Enter opens ",
                        app,
                    ))
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                rect,
                &mut state,
            );
        }
        Modal::Resume(selected) => {
            if app.resume_positions().is_empty() {
                f.render_widget(Paragraph::new("\n  You're all caught up.\n  Saved positions appear here after you start watching.")
                    .wrap(Wrap { trim: true }).block(panel(" Continue watching · Esc closes ", app)), rect);
                return;
            }
            let items = app
                .resume_positions()
                .iter()
                .map(|p| {
                    ListItem::new(vec![
                        Line::from(format!(
                            "{} · {}:{:02}",
                            p.media.title,
                            p.seconds as u64 / 60,
                            p.seconds as u64 % 60
                        )),
                        Line::styled(
                            p.media.creator.clone(),
                            Style::default().fg(themed(app, app.palette.muted)),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(
                List::new(items)
                    .block(panel(
                        " Resume locally · Enter resumes · Del forgets · Esc closes ",
                        app,
                    ))
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                rect,
                &mut state,
            );
        }
        Modal::Destinations(selected) => {
            let items = if app.snapshot.recent_destinations.is_empty() {
                vec![ListItem::new(
                    "No recent folders yet. Choose a destination in download options first.",
                )]
            } else {
                app.snapshot
                    .recent_destinations
                    .iter()
                    .map(|path| ListItem::new(path.display().to_string()))
                    .collect()
            };
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(
                List::new(items)
                    .block(panel(
                        " Recent download folders · Enter reviews options · Esc closes ",
                        app,
                    ))
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                rect,
                &mut state,
            );
        }
        Modal::PlayerSettings { section, selected } => {
            let block = panel(
                " Player selection · ←/→ tab · ↑/↓ choose · Enter applies · Esc closes ",
                app,
            );
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(inner);
            f.render_widget(Paragraph::new(format!(" {} · {}\n Format changes restart this stream at the same position/pause. Loaded tracks switch in place.", ["Available formats", "Loaded audio & subtitles", "Available captions"][*section], app.player_status.backend)).wrap(Wrap { trim: false }), chunks[0]);
            let choices = app.playback_choices(*section);
            if choices.is_empty() {
                f.render_widget(Paragraph::new(" No choices reported by this extractor/player yet. Start playback first.\n Native extraction may expose only a subset; unavailable tracks are never invented.\n Captions require an available VTT/SRT source; DRM streams remain unsupported.").wrap(Wrap { trim: false }), chunks[1]);
            } else {
                let mut state = ListState::default().with_selected(Some(*selected));
                f.render_stateful_widget(
                    List::new(
                        choices
                            .into_iter()
                            .map(|(label, _)| ListItem::new(label))
                            .collect::<Vec<_>>(),
                    )
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                    chunks[1],
                    &mut state,
                );
            }
        }
        Modal::Account { profiles, selected } => {
            let block = panel(
                " YouTube account · select profile · Enter reviews · Esc closes ",
                app,
            );
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let chunks = Layout::vertical([Constraint::Length(6), Constraint::Min(1)]).split(inner);
            f.render_widget(Paragraph::new(" Sign in to YouTube in your browser and complete any consent/verification there.\n Choose a profile below, then confirm the import in the next form.\n Discovery reads directory names only; cookies stay on this computer.\n Keyring must be unlocked. File storage is an explicit Settings → Account option.\n Session expired? Reconnect the selected profile; ytui never accepts consent for you.").wrap(Wrap { trim: false }), chunks[0]);
            let mut items = vec![ListItem::new("Manual profile path / Netscape cookie file…")];
            items.extend(profiles.iter().map(|profile| {
                ListItem::new(vec![
                    Line::from(profile.browser.clone()),
                    Line::styled(
                        profile.path.display().to_string(),
                        Style::default().fg(themed(app, app.palette.muted)),
                    ),
                ])
            }));
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                    .highlight_symbol("▸ "),
                chunks[1],
                &mut state,
            );
        }
        Modal::Outputs { job, selected } => {
            if let Some(download) = app.snapshot.downloads.jobs.iter().find(|j| j.id == *job) {
                if download.outputs.is_empty() {
                    f.render_widget(Paragraph::new(format!("{}\n{}\n\nRetry/select this job in Downloads or its collection group.", download.media.title, download.error.as_deref().unwrap_or("Waiting for completed files; pending work has not published an output yet."))).wrap(Wrap { trim: true }).block(panel(format!(" Files · collection {:?} · Esc closes ", download.state), app)), rect);
                    return;
                }
                let ready = download
                    .outputs
                    .iter()
                    .filter(|path| ytui_services::library::completed_file(download, path).is_some())
                    .count();
                let items = download
                    .outputs
                    .iter()
                    .map(|path| {
                        let available =
                            ytui_services::library::completed_file(download, path).is_some();
                        ListItem::new(vec![
                            Line::styled(
                                format!(
                                    "{}  {}",
                                    if available {
                                        "Ready"
                                    } else {
                                        "Missing / sidecar"
                                    },
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                                Style::default().fg(themed(
                                    app,
                                    if available {
                                        app.palette.text
                                    } else {
                                        app.palette.muted
                                    },
                                )),
                            ),
                            Line::styled(
                                path.parent().unwrap_or(path).display().to_string(),
                                Style::default().fg(themed(app, app.palette.muted)),
                            ),
                        ])
                    })
                    .collect::<Vec<_>>();
                let mut state = ListState::default().with_selected(Some(*selected));
                f.render_stateful_widget(
                    List::new(items)
                        .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                        .highlight_symbol("▸ ")
                        .block(
                            panel(
                                format!(
                                    " Files · {ready} ready · collection {:?} ",
                                    download.state
                                ),
                                app,
                            )
                            .title_bottom(" ↑/↓ select · Enter plays · o Open with… · Esc closes "),
                        ),
                    rect,
                    &mut state,
                );
            }
        }
        Modal::Queue(selected) => {
            let items = app
                .snapshot
                .playback
                .items
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    ListItem::new(format!(
                        "{} {} · {}",
                        if app.snapshot.playback.current == Some(i) {
                            "▶"
                        } else {
                            " "
                        },
                        m.title,
                        m.creator
                    ))
                })
                .collect::<Vec<_>>();
            let list = List::new(items)
                .block(panel(
                    " Playback queue · Enter plays · Del removes · Esc closes ",
                    app,
                ))
                .highlight_style(Style::default().bg(themed(app, app.palette.selection)))
                .highlight_symbol("▸ ");
            let mut state = ListState::default().with_selected(Some(*selected));
            f.render_stateful_widget(list, rect, &mut state);
        }
        Modal::Form(form) => {
            let block = panel(format!(" {} ", app.tr(form.title())), app).title_bottom(
                Line::from(format!(
                    " {} / {} fields ",
                    form.selected + 1,
                    form.fields.len()
                ))
                .right_aligned(),
            );
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let visible = inner.height.saturating_sub(5) as usize;
            let start = form.selected.saturating_sub(visible.saturating_sub(1));
            for (index, field) in form.fields.iter().enumerate().skip(start).take(visible) {
                let row = Rect::new(
                    inner.x + 1,
                    inner.y + (index - start) as u16,
                    inner.width.saturating_sub(2),
                    1,
                );
                let label_width = if row.width > 65 { 27 } else { row.width / 2 };
                let active = index == form.selected;
                let style = Style::default()
                    .bg(themed(
                        app,
                        if active {
                            app.palette.selection
                        } else {
                            app.palette.background
                        },
                    ))
                    .fg(themed(
                        app,
                        if active {
                            app.palette.accent
                        } else {
                            app.palette.text
                        },
                    ));
                f.render_widget(
                    Paragraph::new(format!(
                        "{:<width$}",
                        app.tr(field.label),
                        width = label_width as usize
                    ))
                    .style(style),
                    Rect::new(row.x, row.y, label_width.saturating_sub(1), 1),
                );
                let width = row.width.saturating_sub(label_width + 1);
                let (text, cursor) = viewport(
                    &field.value.text,
                    field.value.cursor,
                    width.saturating_sub(1),
                );
                let value_area = Rect::new(row.x + label_width, row.y, width, 1);
                let text = if field.toggle {
                    if field.value.text == "yes" {
                        app.tr("● on").into()
                    } else {
                        app.tr("○ off").into()
                    }
                } else {
                    text
                };
                f.render_widget(Paragraph::new(text).style(style), value_area);
                if active && !field.toggle {
                    f.set_cursor_position((value_area.x + cursor, value_area.y));
                }
            }
            let bottom = inner.bottom().saturating_sub(4);
            let message = form
                .error
                .as_deref()
                .unwrap_or("↑/↓ fields · Space toggles · Enter saves · Esc cancels");
            f.render_widget(
                Paragraph::new(app.tr(message))
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(themed(
                        app,
                        if form.error.is_some() {
                            app.palette.error
                        } else {
                            app.palette.muted
                        },
                    ))),
                Rect::new(inner.x + 1, bottom, inner.width.saturating_sub(2), 2),
            );
            let hint = if matches!(form.kind, crate::forms::FormKind::Keys(_)) {
                "Names: enter, space, ctrl+r, f8 · Separate multiple keys with spaces"
            } else if matches!(form.kind, crate::forms::FormKind::Auth) {
                "Only the selected profile is read. Sign in to YouTube there first."
            } else if matches!(form.kind, crate::forms::FormKind::Settings(_)) {
                "Windows: HH:MM-HH:MM (local time); blank=off · rates in total KiB/s"
            } else if matches!(
                form.kind,
                crate::forms::FormKind::Action {
                    command: "lyrics-find",
                    ..
                }
            ) {
                "Enter sends only the reviewed title and artist to LRCLIB. No cookies or history."
            } else if matches!(
                form.kind,
                crate::forms::FormKind::Action {
                    command: "lyrics-local",
                    ..
                }
            ) {
                "Open a local UTF-8 .lrc or text file up to 512 KiB. No network request."
            } else if matches!(
                form.kind,
                crate::forms::FormKind::Action {
                    command: "sleep",
                    ..
                }
            ) {
                "30m / 1h / 90s · max 24h · Stops playback; keeps queue · Session only"
            } else if matches!(
                form.kind,
                crate::forms::FormKind::Action {
                    command: "saved",
                    ..
                }
            ) {
                "Search local titles, artists or video IDs · Blank shows all saved media"
            } else {
                "Playlist indices: 1,3:7 · Quality: best or height (e.g. 1080)"
            };
            f.render_widget(
                Paragraph::new(app.tr(hint))
                    .style(Style::default().fg(themed(app, app.palette.muted))),
                Rect::new(inner.x + 1, bottom + 2, inner.width.saturating_sub(2), 1),
            );
        }
    }
}
fn theme_items(app: &App) -> Vec<ListItem<'static>> {
    ytui_services::theme::THEMES
        .iter()
        .map(|name| {
            ListItem::new(format!(
                "{} {name}",
                if *name == app.config.theme {
                    "●"
                } else {
                    " "
                }
            ))
        })
        .collect()
}
pub fn viewport(text: &str, cursor: usize, width: u16) -> (String, u16) {
    let mut start = 0;
    let mut cursor_width = text[..cursor].width();
    for (index, c) in text[..cursor].char_indices() {
        if cursor_width < width as usize {
            break;
        }
        start = index + c.len_utf8();
        cursor_width = cursor_width.saturating_sub(c.width().unwrap_or(0));
    }
    (
        text[start..].to_owned(),
        cursor_width.min(width as usize) as u16,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_viewport_uses_display_width() {
        let (s, c) = viewport("日本語abc", 9, 5);
        assert_eq!(s, "本語abc");
        assert_eq!(c, 4);
    }
}
