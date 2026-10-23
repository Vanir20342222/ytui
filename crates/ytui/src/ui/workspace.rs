//! The main workspace: compact navigation, a readable catalog and selected-item details.
use super::*;
use crate::app::HitTarget;
use ytui_core::{Job, shortcuts::Action};

fn style(app: &App, color: (u8, u8, u8)) -> Style {
    Style::default().fg(themed(app, color))
}
pub(super) fn key(app: &App, action: Action) -> String {
    app.config
        .keybindings
        .iter()
        .find(|(_, value)| value.as_str() == action.name())
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| {
            REGISTRY
                .iter()
                .find(|(_, a, _, _)| *a == action)
                .filter(|(default, _, _, _)| !app.config.keybindings.contains_key(*default))
                .map(|(k, _, _, _)| *k)
                .unwrap_or(if action == Action::Feeds {
                    ":feeds"
                } else {
                    "unbound"
                })
                .into()
        })
}
fn hints(app: &App, entries: &[(Action, &str)], width: u16) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0;
    for (action, label) in entries {
        let key = key(app, *action);
        let length = key.width() + label.width() + 4;
        if used + length > width as usize {
            break;
        }
        used += length;
        spans.push(Span::styled(
            format!(" {key} "),
            style(app, app.palette.accent).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{label}  "),
            style(app, app.palette.muted),
        ));
    }
    Line::from(spans)
}
fn inset(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    )
}
pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.image_area = Rect::default();
    app.card_areas.clear();
    app.hit_targets.clear();
    f.render_widget(
        Block::default()
            .style(style(app, app.palette.text).bg(themed(app, app.palette.background))),
        area,
    );
    if area.width < 35 || area.height < 12 {
        f.render_widget(
            Paragraph::new("ytui\nEnlarge to at least 35 × 12.\nCtrl+C quits safely.")
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    app.sidebar_visible = area.width >= 140 && area.height >= 26;
    if app.sidebar_visible {
        wide_workspace(f, app, area);
    } else {
        app.feeds_focused = false;
        compact_workspace(f, app, area);
    }
    if app.modal.is_some() || app.input.is_some() {
        for cell in &mut f.buffer_mut().content {
            let fade = |color| match color {
                Color::Rgb(r, g, b) => {
                    let (br, bg, bb) = app.palette.background;
                    Color::Rgb(
                        ((r as u16 * 2 + br as u16 * 3) / 5) as u8,
                        ((g as u16 * 2 + bg as u16 * 3) / 5) as u8,
                        ((b as u16 * 2 + bb as u16 * 3) / 5) as u8,
                    )
                }
                _ => Color::DarkGray,
            };
            cell.set_fg(fade(cell.fg));
            cell.set_bg(if app.palette.name == "terminal" {
                Color::Reset
            } else {
                fade(cell.bg)
            });
        }
        if app.modal.is_some() {
            modal(f, app, area);
        } else {
            super::menus::search(f, app, centered(area, 90, 8));
        }
    }
}
fn compact_workspace(f: &mut Frame, app: &mut App, area: Rect) {
    let compact = area.height < 22;
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(if area.height < 18 {
            1
        } else if !compact && (app.player_status.active || app.player_status.error.is_some()) {
            6
        } else {
            3
        }),
        Constraint::Length(if app.config.show_hints { 2 } else { 1 }),
    ])
    .split(area);
    header(f, app, chunks[0]);
    let tabs = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(chunks[1]);
    for (index, name) in TAB_NAMES.iter().enumerate() {
        let active = index == app.tab;
        let name = if area.width < 60 {
            ["Downloads", "YouTube", "Music", "Radio"][index]
        } else {
            name
        };
        let label = if area.width < 48 {
            format!(
                "{} {}",
                index + 1,
                ["Files", "Videos", "Music", "Radio"][index]
            )
        } else {
            format!("{}  {name}", index + 1)
        };
        let tab = tabs[index];
        app.hit_targets.push((tab, HitTarget::Tab(index)));
        f.render_widget(
            Paragraph::new(label)
                .centered()
                .style(
                    style(
                        app,
                        if active {
                            app.palette.accent
                        } else {
                            app.palette.muted
                        },
                    )
                    .bg(themed(app, app.palette.background))
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                )
                .block(
                    Block::default()
                        .borders(Borders::BOTTOM)
                        .border_style(style(
                            app,
                            if active {
                                app.palette.accent
                            } else {
                                app.palette.surface
                            },
                        )),
                ),
            tab,
        );
    }
    if app.tab == 0 {
        downloads(f, app, chunks[2]);
    } else {
        browse(f, app, chunks[2]);
    }
    playback(f, app, chunks[3]);
    footer(f, app, chunks[4]);
}
fn wide_workspace(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Min(10),
        Constraint::Length(if app.config.show_hints { 2 } else { 1 }),
    ])
    .split(area);
    let cols = Layout::horizontal([Constraint::Length(34), Constraint::Min(40)]).split(rows[0]);
    sidebar(f, app, cols[0]);
    let main = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(
            if app.player_status.active || app.player_status.error.is_some() {
                6
            } else {
                3
            },
        ),
    ])
    .split(cols[1]);
    if app.tab == 0 {
        downloads(f, app, main[0]);
    } else {
        browse(f, app, main[0]);
    }
    playback(f, app, main[1]);
    footer(f, app, rows[1]);
}
fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Min(5),
    ])
    .split(area);
    let block = panel(" ytui · Status ", app);
    let inner = inset(block.inner(rows[0]));
    f.render_widget(block, rows[0]);
    let account = app
        .session
        .as_ref()
        .map(|s| s.account.as_str())
        .unwrap_or(if app.offline {
            "Offline"
        } else if app.demo {
            "Demo · fictional catalog"
        } else {
            "Anonymous"
        });
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(account.to_owned(), style(app, app.palette.success)),
            Line::from(format!(
                "{} · {} jobs",
                TAB_NAMES[app.tab],
                app.snapshot.downloads.jobs.len()
            )),
            Line::styled(app.palette.name.clone(), style(app, app.palette.muted)),
        ]),
        inner,
    );
    let block = panel(" Workspaces · 1–4 ", app);
    let inner = block.inner(rows[1]);
    f.render_widget(block, rows[1]);
    for (index, name) in TAB_NAMES.iter().enumerate() {
        let active = app.tab == index;
        let row = Rect::new(inner.x, inner.y + index as u16, inner.width, 1);
        app.hit_targets.push((row, HitTarget::Tab(index)));
        f.render_widget(
            Paragraph::new(format!(
                " {} {}  {name}",
                if active { "▸" } else { " " },
                index + 1
            ))
            .style(
                style(
                    app,
                    if active {
                        app.palette.accent
                    } else {
                        app.palette.text
                    },
                )
                .bg(themed(
                    app,
                    if active {
                        app.palette.selection
                    } else {
                        app.palette.background
                    },
                ))
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
            row,
        );
    }
    let block = panel(format!(" Feeds · {} ", key(app, Action::Feeds)), app)
        .border_style(style(
            app,
            if app.feeds_focused {
                app.palette.accent
            } else {
                app.palette.muted
            },
        ))
        .title_bottom(Line::from(if app.feeds_focused {
            " Enter opens · Esc returns "
        } else {
            " Select a source "
        }));
    let inner = block.inner(rows[2]);
    f.render_widget(block, rows[2]);
    app.hit_targets.push((rows[2], HitTarget::Feeds));
    let visible = inner.height.saturating_sub(1) as usize;
    let first = app.feed_selected.saturating_sub(visible.saturating_sub(1));
    for (index, (label, _, _)) in app
        .feed_choices()
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
    {
        let row = Rect::new(inner.x, inner.y + (index - first) as u16, inner.width, 1);
        let active = app.feeds_focused && app.feed_selected == index;
        f.render_widget(
            Paragraph::new(format!(" {} {label}", if active { "▸" } else { " " })).style(
                style(
                    app,
                    if active {
                        app.palette.accent
                    } else {
                        app.palette.text
                    },
                )
                .bg(themed(
                    app,
                    if active {
                        app.palette.selection
                    } else {
                        app.palette.background
                    },
                )),
            ),
            row,
        );
        app.hit_targets.push((row, HitTarget::Feed(index)));
    }
}
fn header(f: &mut Frame, app: &App, area: Rect) {
    let account = app
        .session
        .as_ref()
        .map(|s| s.account.as_str())
        .unwrap_or(if app.offline {
            "OFFLINE"
        } else if app.demo {
            "DEMO"
        } else {
            "Guest"
        });
    let right_width = (account.width().min(24) as u16 + 4).min(area.width / 3);
    let cols =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(right_width)]).split(area);
    let spans = vec![
        Span::styled(
            " ytui ",
            style(app, app.palette.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("· {}", TAB_NAMES[app.tab]),
            style(app, app.palette.muted),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(spans)), cols[0]);
    f.render_widget(
        Paragraph::new(format!(" {account} "))
            .right_aligned()
            .style(style(app, app.palette.muted)),
        cols[1],
    );
}
fn footer(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let cols = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(if area.width >= 80 { 47 } else { 27 }),
    ])
    .split(rows[0]);
    let buttons: &[(Action, &str)] = if area.width >= 80 {
        &[
            (Action::Command, "Actions"),
            (Action::Settings, "Settings"),
            (Action::PlayerMenu, "Player"),
            (Action::ToggleHints, "Hints"),
        ]
    } else {
        &[
            (Action::Command, "Menu"),
            (Action::Settings, "Settings"),
            (Action::ToggleHints, "Hints"),
        ]
    };
    let mut x = cols[1].x;
    for (action, label) in buttons {
        let label = format!(" {} {label} ", key(app, *action));
        let width = (label.width() as u16).min(cols[1].right().saturating_sub(x));
        let rect = Rect::new(x, cols[1].y, width, 1);
        f.render_widget(
            Paragraph::new(label).style(style(app, app.palette.accent)),
            rect,
        );
        app.hit_targets.push((rect, HitTarget::Action(*action)));
        x += width;
    }
    f.render_widget(
        Paragraph::new(format!(
            " {}",
            app.pending_operation
                .as_ref()
                .map(|(_, text)| text.as_str())
                .unwrap_or(if app.player_session_pending {
                    "Updating player account · playback waits for the account handoff"
                } else if app.collection_pending {
                    "Loading full collection · Actions → Cancel collection loading"
                } else {
                    &app.status
                })
        ))
        .style(style(app, app.palette.muted)),
        cols[0],
    );
    if !app.config.show_hints {
        return;
    }
    let mut entries = if app.feeds_focused {
        vec![
            (Action::Up, "previous"),
            (Action::Down, "next"),
            (Action::Open, "open feed"),
            (Action::Back, "return"),
        ]
    } else if app.tab == 0 {
        vec![
            (Action::Search, "add"),
            (Action::Pause, "pause"),
            (Action::Feeds, "library"),
            (Action::Download, "options"),
            (Action::Retry, "retry"),
            (Action::Remove, "remove"),
        ]
    } else if app.tab == 3 {
        vec![
            (Action::Open, "play/open"),
            (Action::Search, "stations"),
            (Action::Feeds, "sources"),
            (Action::Favorite, "save"),
            (Action::PlaybackQueue, "queue"),
            (Action::Back, "back"),
        ]
    } else if app.selected().is_some_and(|m| m.collection()) {
        vec![
            (Action::Open, "open"),
            (Action::Search, "search"),
            (Action::Feeds, "feeds"),
            (Action::Download, "download"),
            (Action::Back, "back"),
        ]
    } else {
        vec![
            (Action::Open, "play/open"),
            (Action::Search, "search"),
            (Action::Feeds, "feeds"),
            (Action::Preview, "preview"),
            (Action::Download, "download"),
            (Action::Back, "back"),
        ]
    };
    // Keep help visible even when the more detailed action list will not fit.
    entries.insert(2, (Action::Help, "help"));
    if area.width < 45 {
        entries = vec![
            (Action::Search, if app.tab == 0 { "add" } else { "search" }),
            (Action::Feeds, "feeds"),
            (Action::Help, "help"),
        ];
    }
    f.render_widget(Paragraph::new(hints(app, &entries, area.width)), rows[1]);
}
fn job_state(app: &App, job: &Job) -> (&'static str, (u8, u8, u8)) {
    match job.state {
        JobState::Done => ("Complete", app.palette.success),
        JobState::Error => ("Failed", app.palette.error),
        JobState::Paused => ("Paused", app.palette.warning),
        JobState::Cancelled => ("Cancelled", app.palette.muted),
        JobState::Queued
            if job
                .not_before
                .is_some_and(|at| at > ytui_core::schedule::unix_now()) =>
        {
            ("Scheduled", app.palette.warning)
        }
        JobState::Queued => ("Queued", app.palette.muted),
        JobState::Resolving => ("Resolving", app.palette.accent),
        JobState::Downloading => ("Downloading", app.palette.accent),
        JobState::Merging => ("Merging", app.palette.accent),
        JobState::Converting => ("Converting", app.palette.accent),
        JobState::Backoff => ("Retry wait", app.palette.warning),
    }
}
fn meter(value: f64, width: usize) -> String {
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = (value * width as f64).round() as usize;
    format!("{}{}", "━".repeat(filled), "─".repeat(width - filled))
}
fn downloads(f: &mut Frame, app: &mut App, area: Rect) {
    let jobs = &app.snapshot.downloads.jobs;
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
    let mut summary = Vec::new();
    for (label, count, color) in [
        (
            "active",
            jobs.iter().filter(|j| j.state.active()).count(),
            app.palette.accent,
        ),
        (
            "queued",
            jobs.iter().filter(|j| j.state == JobState::Queued).count(),
            app.palette.muted,
        ),
        (
            "paused",
            jobs.iter().filter(|j| j.state == JobState::Paused).count(),
            app.palette.warning,
        ),
        (
            "failed",
            jobs.iter().filter(|j| j.state == JobState::Error).count(),
            app.palette.error,
        ),
        (
            "complete",
            jobs.iter().filter(|j| j.state == JobState::Done).count(),
            app.palette.success,
        ),
    ] {
        if area.width < 70 && label == "paused" {
            continue;
        }
        let label = if area.width < 70 {
            match label {
                "active" => "run",
                "queued" => "wait",
                "failed" => "fail",
                _ => "done",
            }
        } else {
            label
        };
        summary.push(Span::styled(
            format!(" {count} {label} "),
            style(app, color),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(summary)), chunks[0]);
    if jobs.is_empty() {
        empty(
            f,
            app,
            chunks[1],
            "Your download queue is clear",
            &format!(
                "{}  Add a video or playlist URL\n{}  Browse completed downloads and your library",
                key(app, Action::Search),
                key(app, Action::Feeds)
            ),
        );
        return;
    }
    let wide = area.width >= 115 && area.height >= 17;
    let cols = Layout::horizontal([
        Constraint::Percentage(if wide { 68 } else { 100 }),
        Constraint::Min(0),
    ])
    .split(chunks[1]);
    let block = panel(" Download queue ", app)
        .border_style(style(
            app,
            if app.feeds_focused {
                app.palette.muted
            } else {
                app.palette.accent
            },
        ))
        .title_bottom(
            Line::from(format!(" {} / {} ", app.download_selected + 1, jobs.len())).right_aligned(),
        );
    let rows = jobs.iter().map(|job| {
        let (label, color) = job_state(app, job);
        let title = if job.media.title.is_empty() {
            &job.media.url
        } else {
            &job.media.title
        };
        let subtitle = if job.state == JobState::Error {
            job.error.clone().unwrap_or_default()
        } else {
            format!(
                "{} · {}",
                if job.options.audio {
                    &job.options.audio_container
                } else {
                    &job.options.container
                },
                if job.options.audio {
                    &job.options.audio_quality
                } else {
                    &job.options.quality
                }
            )
        };
        let progress = if job.state == JobState::Done {
            "100%".into()
        } else {
            job.progress
                .fraction()
                .map(|p| format!("{:>3.0}%", p * 100.0))
                .unwrap_or_else(|| "—".into())
        };
        let transfer = job
            .progress
            .speed
            .filter(|n| n.is_finite())
            .map(|n| format!("{:.1} MiB/s", n / 1048576.0))
            .unwrap_or_default();
        Row::new(vec![
            Cell::from(label).style(style(app, color)),
            Cell::from(Text::from(vec![
                Line::from(title.clone()),
                Line::styled(
                    subtitle,
                    style(
                        app,
                        if job.state == JobState::Error {
                            app.palette.error
                        } else {
                            app.palette.muted
                        },
                    ),
                ),
            ])),
            Cell::from(Text::from(vec![
                Line::from(progress),
                Line::styled(transfer, style(app, app.palette.muted)),
            ])),
        ])
        .height(2)
    });
    let list_inner = block.inner(cols[0]);
    let mut table = Table::new(
        rows,
        [
            Constraint::Length(if area.width < 60 { 9 } else { 11 }),
            Constraint::Min(6),
            Constraint::Length(if area.width < 75 { 5 } else { 12 }),
        ],
    )
    .block(block)
    .row_highlight_style(Style::default().bg(themed(app, app.palette.selection)))
    .highlight_symbol("▎ ")
    .column_spacing(1);
    if cols[0].height >= 8 {
        table = table.header(
            Row::new(["STATUS", "TITLE / FORMAT", "PROGRESS"])
                .style(style(app, app.palette.muted))
                .height(2),
        );
    }
    let mut state = TableState::default().with_selected(Some(app.download_selected));
    f.render_stateful_widget(table, cols[0], &mut state);
    app.hit_targets.push((cols[0], HitTarget::Catalog));
    let header_height = if cols[0].height >= 8 { 2 } else { 0 };
    let visible = list_inner.height.saturating_sub(header_height) / 2;
    for index in state.offset()..(state.offset() + visible as usize).min(jobs.len()) {
        app.hit_targets.push((
            Rect::new(
                list_inner.x,
                list_inner.y + header_height + (index - state.offset()) as u16 * 2,
                list_inner.width,
                2,
            ),
            HitTarget::Item(index),
        ));
    }
    if wide && let Some(job) = jobs.get(app.download_selected) {
        download_details(f, app, job, cols[1]);
    }
}
fn download_details(f: &mut Frame, app: &App, job: &Job, area: Rect) {
    let block = panel(" Selected download ", app);
    let inner = inset(block.inner(area));
    f.render_widget(block, area);
    let (label, color) = job_state(app, job);
    let mut lines = vec![Line::styled(
        label,
        style(app, color).add_modifier(Modifier::BOLD),
    )];
    if let Some(error) = &job.error {
        if let Some(recovery) = ytui_services::failure::recovery(error) {
            lines.extend([
                Line::styled(recovery.title, style(app, app.palette.error)),
                Line::from(recovery.instruction),
            ]);
        }
        lines.push(Line::styled(error.clone(), style(app, app.palette.error)));
    }
    lines.extend([
        Line::from(""),
        Line::styled(
            job.media.title.clone(),
            style(app, app.palette.text).add_modifier(Modifier::BOLD),
        ),
        Line::styled(job.media.creator.clone(), style(app, app.palette.muted)),
        Line::from(""),
    ]);
    if let Some(value) = job.progress.fraction() {
        lines.push(Line::styled(
            format!(
                "{}  {:.0}%",
                meter(value, inner.width.saturating_sub(7) as usize),
                value * 100.0
            ),
            style(app, color),
        ));
    }
    if let Some(eta) = job.progress.eta.filter(|n| n.is_finite()) {
        lines.push(Line::from(format!("{} remaining", time(eta))));
    }
    lines.extend([
        Line::from(""),
        Line::styled("FORMAT", style(app, app.palette.muted)),
        Line::from(format!(
            "{} · {}",
            if job.options.audio {
                &job.options.audio_container
            } else {
                &job.options.container
            },
            if job.options.audio {
                &job.options.audio_quality
            } else {
                &job.options.quality
            }
        )),
        Line::from(""),
        Line::styled("SAVED TO", style(app, app.palette.muted)),
        Line::from(
            job.outputs
                .last()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| job.options.output_directory().display().to_string()),
        ),
    ]);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}
fn empty(f: &mut Frame, app: &App, area: Rect, title: &str, message: &str) {
    let block = panel("", app);
    let inner = inset(block.inner(area));
    f.render_widget(block, area);
    let lines = vec![
        Line::from(""),
        Line::styled(
            title.to_owned(),
            style(app, app.palette.text).add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::styled(message.to_owned(), style(app, app.palette.muted)),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}
fn browse(f: &mut Frame, app: &mut App, area: Rect) {
    let view = &app.tabs[app.tab];
    let category_rows = if view.page.categories.is_empty() {
        2
    } else {
        3
    };
    let chunks =
        Layout::vertical([Constraint::Length(category_rows), Constraint::Min(1)]).split(area);
    if !view.page.categories.is_empty() {
        let mut x = area.x + 1;
        for (index, category) in view.page.categories.iter().enumerate() {
            let width = (category.title.width() as u16 + 4).min(area.width.saturating_sub(2));
            if x + width > area.right() {
                break;
            }
            let rect = Rect::new(x, area.y + 2, width, 1);
            f.render_widget(
                Paragraph::new(format!(" {} ", category.title))
                    .style(style(app, app.palette.accent).bg(themed(app, app.palette.surface))),
                rect,
            );
            app.hit_targets.push((rect, HitTarget::Category(index)));
            x += width + 1;
        }
    }
    let heading = Layout::horizontal([
        Constraint::Min(10),
        Constraint::Length(if area.width > 65 { 23 } else { 10 }),
    ])
    .split(chunks[0]);
    f.render_widget(
        Paragraph::new(format!(
            " {}{}",
            view.page.title,
            if view.loading {
                " · Loading…"
            } else if view
                .load
                .as_ref()
                .is_some_and(|load| load.retry_at.is_some())
            {
                " · Reconnecting automatically…"
            } else {
                ""
            }
        ))
        .style(style(app, app.palette.text).add_modifier(Modifier::BOLD)),
        heading[0],
    );
    f.render_widget(
        Paragraph::new(format!("{}  Browse feeds ", key(app, Action::Feeds)))
            .right_aligned()
            .style(style(app, app.palette.accent)),
        heading[1],
    );
    let notice = view
        .error
        .as_deref()
        .or(view.page.notice.as_deref())
        .unwrap_or(if app.tab == 3 {
            "Streams, track libraries and stations for your day."
        } else {
            "Choose something good. Play it, preview it, or keep it."
        });
    f.render_widget(
        Paragraph::new(format!(" {notice}")).style(style(
            app,
            if view.error.is_some() {
                app.palette.error
            } else {
                app.palette.muted
            },
        )),
        Rect::new(chunks[0].x, chunks[0].y + 1, chunks[0].width, 1),
    );
    if view.page.items.is_empty() {
        empty(
            f,
            app,
            chunks[1],
            if view.loading {
                "Loading your feed…"
            } else if view.error.is_some() {
                "This feed couldn't load"
            } else {
                "Nothing here yet"
            },
            &format!(
                "{notice}\n\n{}\n{}  Choose another feed\n{}  Search for a video, artist or station\n{}  Refresh this page",
                if view
                    .load
                    .as_ref()
                    .is_some_and(|load| load.retry_at.is_some())
                {
                    "Reconnecting automatically. No refresh needed."
                } else {
                    ""
                },
                key(app, Action::Feeds),
                key(app, Action::Search),
                key(app, Action::Refresh)
            ),
        );
        return;
    }
    let wide = area.width >= 100 && area.height >= 17;
    let cols = Layout::horizontal([
        Constraint::Percentage(if wide { 74 } else { 100 }),
        Constraint::Min(0),
    ])
    .split(chunks[1]);
    let block = panel(
        if app.tab == 3 {
            " Stations & channels "
        } else {
            " Browse "
        },
        app,
    )
    .border_style(style(
        app,
        if app.feeds_focused {
            app.palette.muted
        } else {
            app.palette.accent
        },
    ))
    .title_bottom(
        Line::from(format!(
            " {} / {}{} ",
            view.selected + 1,
            view.page.items.len(),
            if view.page.continuation.is_some() {
                format!(" · {} more", key(app, Action::NextPage))
            } else {
                String::new()
            }
        ))
        .right_aligned(),
    );
    let list_area = block.inner(cols[0]);
    f.render_widget(block, cols[0]);
    app.hit_targets.push((cols[0], HitTarget::Catalog));
    let cards = area.height >= 24 && list_area.width >= 65 && app.tab != 3;
    let height = if cards {
        8
    } else if list_area.height < 12 {
        3u16
    } else {
        4
    };
    let columns = if cards && list_area.width >= 110 {
        2
    } else {
        1usize
    };
    let visible = (list_area.height / height).max(1) as usize;
    let first = (view.selected / columns).saturating_sub(visible - 1) * columns;
    for (index, item) in view
        .page
        .items
        .iter()
        .enumerate()
        .skip(first)
        .take(visible * columns)
    {
        let local = index - first;
        let width = list_area.width / columns as u16;
        let row = Rect::new(
            list_area.x + (local % columns) as u16 * width,
            list_area.y + (local / columns) as u16 * height,
            width,
            height.min(list_area.height),
        );
        app.hit_targets.push((row, HitTarget::Item(index)));
        let active = index == view.selected;
        let bg = themed(
            app,
            if active {
                app.palette.selection
            } else {
                app.palette.background
            },
        );
        f.render_widget(Block::default().style(Style::default().bg(bg)), row);
        let marker = if active { "▎" } else { " " };
        f.render_widget(
            Paragraph::new(vec![Line::from(marker); height as usize])
                .style(style(app, app.palette.accent)),
            Rect::new(row.x, row.y, 1, row.height),
        );
        let favorite = app.snapshot.favorites.iter().any(|m| m.url == item.url);
        let meta = format!(
            "{}{}{}",
            item.label(),
            if item.tracks_url.is_some() {
                " · track library"
            } else {
                ""
            },
            if favorite { " · saved" } else { "" }
        );
        let text_x = if cards
            && app.config.thumbnails
            && app.config.image_protocol != "none"
            && !item.thumbnail.is_empty()
        {
            let (fw, fh) = app.picker.font_size();
            let aspect = if app.tab == 2 { 1.0 } else { 16.0 / 9.0 };
            let image_width =
                ((height - 1) as f64 * fh as f64 * aspect / fw.max(1) as f64).round() as u16;
            let image_width = image_width.min(row.width / 2).max(1);
            let image_area = Rect::new(row.x + 2, row.y, image_width, height - 1);
            app.card_areas.push((index, image_area));
            let key = app.card_key(item, image_area);
            if app.modal.is_none()
                && let Some(image) = app.card_images.get(&key)
            {
                f.render_widget(Image::new(image), image_area);
            } else {
                f.render_widget(
                    Block::default().style(Style::default().bg(themed(app, app.palette.surface))),
                    image_area,
                );
                if let Some(notice) = app.card_notice(&key) {
                    f.render_widget(
                        Paragraph::new(notice)
                            .centered()
                            .wrap(Wrap { trim: true })
                            .style(style(app, app.palette.muted)),
                        image_area,
                    );
                }
            }
            image_area.right() + 2
        } else {
            row.x + 2
        };
        let section = if item.section.is_empty() {
            String::new()
        } else {
            format!(" · {}", item.section)
        };
        let meta = format!("{meta}{section}");
        let lines = if height <= 3 {
            vec![
                Line::styled(
                    item.title.clone(),
                    style(app, app.palette.text).add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    format!("{} · {meta}", item.creator),
                    style(app, app.palette.muted),
                ),
            ]
        } else {
            vec![
                Line::styled(
                    item.title.clone(),
                    style(app, app.palette.text).add_modifier(Modifier::BOLD),
                ),
                Line::styled(item.creator.clone(), style(app, app.palette.muted)),
                Line::styled(
                    meta,
                    style(
                        app,
                        if active {
                            app.palette.accent
                        } else {
                            app.palette.muted
                        },
                    ),
                ),
            ]
        };
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(bg)),
            Rect::new(
                text_x,
                row.y,
                row.right().saturating_sub(text_x + 1),
                row.height,
            ),
        );
    }
    if wide {
        selected_details(f, app, cols[1]);
    }
}
fn selected_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(" In focus ", app);
    let inner = inset(block.inner(area));
    f.render_widget(block, area);
    let Some(media) = app.selected() else {
        return;
    };
    let show_image = app.config.thumbnails
        && app.config.image_protocol != "none"
        && !media.thumbnail.is_empty()
        && app.modal.is_none();
    let (font_width, font_height) = app.picker.font_size();
    let aspect = if app.tab == 2 { 1.0 } else { 16.0 / 9.0 };
    let image_height = if show_image {
        (inner.width as f64 * font_width as f64 / font_height.max(1) as f64 / aspect).round() as u16
    } else {
        0
    };
    let image_height = if show_image {
        image_height.min(14).min(inner.height.saturating_sub(8))
    } else {
        0
    };
    let text_y = if image_height > 0 {
        let (font_width, font_height) = app.picker.font_size();
        let image_width = ((image_height as f64 * font_height as f64 * aspect
            / font_width.max(1) as f64)
            .round() as u32)
            .min(inner.width as u32) as u16;
        let rect = Rect::new(
            inner.x + (inner.width - image_width) / 2,
            inner.y,
            image_width,
            image_height,
        );
        app.image_area = rect;
        if let Some(image) = &app.image {
            f.render_widget(Image::new(image), rect);
        } else {
            f.render_widget(
                Paragraph::new(app.image_error.as_deref().unwrap_or(""))
                    .centered()
                    .style(style(app, app.palette.muted).bg(themed(app, app.palette.surface))),
                rect,
            );
        }
        rect.bottom() + 1
    } else {
        inner.y + 1
    };
    let regions = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).split(Rect::new(
        inner.x,
        text_y,
        inner.width,
        inner.bottom().saturating_sub(text_y),
    ));
    let mut lines = vec![
        Line::styled(
            media.title.clone(),
            style(app, app.palette.text).add_modifier(Modifier::BOLD),
        ),
        Line::styled(media.creator.clone(), style(app, app.palette.accent)),
        Line::from(""),
        Line::styled(media.label(), style(app, app.palette.muted)),
        Line::from(""),
    ];
    lines.push(Line::from(if media.description.is_empty() {
        if media.collection() {
            "Open this collection to explore its videos or tracks.".into()
        } else if app.tab == 3 {
            "Play this station or save it to your favorites.".into()
        } else if media.kind == ytui_core::MediaKind::Song || app.tab == 2 {
            "Play this track or add it to your queue.".into()
        } else {
            "Preview this video, play it now, or add it to your downloads.".into()
        }
    } else {
        media.description.clone()
    }));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), regions[0]);
    let actions = if media.kind == ytui_core::MediaKind::Station {
        vec![
            hints(app, &[(Action::Open, "play/open")], regions[1].width),
            hints(app, &[(Action::Favorite, "save station")], regions[1].width),
            hints(app, &[(Action::Enqueue, "queue")], regions[1].width),
        ]
    } else if media.collection() {
        vec![
            hints(app, &[(Action::Open, "open collection")], regions[1].width),
            hints(
                app,
                &[(Action::Download, "download playlist")],
                regions[1].width,
            ),
            Line::styled(
                " F2 → Library → Watch collection",
                style(app, app.palette.muted),
            ),
        ]
    } else {
        vec![
            hints(app, &[(Action::Open, "play now")], regions[1].width),
            hints(
                app,
                &[(Action::Preview, "preview"), (Action::Download, "download")],
                regions[1].width,
            ),
            hints(
                app,
                &[(Action::Enqueue, "queue"), (Action::Resume, "resume")],
                regions[1].width,
            ),
        ]
    };
    f.render_widget(Paragraph::new(actions), regions[1]);
}
fn playback(f: &mut Frame, app: &mut App, area: Rect) {
    let status = &app.player_status;
    if status.preview {
        let message = format!(" Video preview · 10s · {} closes", key(app, Action::Back));
        let widget = Paragraph::new(message).style(style(app, app.palette.accent));
        f.render_widget(
            if area.height > 1 {
                widget.block(panel(" Preview ", app))
            } else {
                widget
            },
            area,
        );
        return;
    }
    if area.height == 1 {
        if let Some(error) = &status.error {
            let title =
                ytui_services::failure::recovery(error).map_or("Playback failed", |r| r.title);
            f.render_widget(
                Paragraph::new(format!(" {} Actions · {title}", key(app, Action::Command)))
                    .style(style(app, app.palette.error)),
                area,
            );
        } else if status.active {
            let cols = Layout::horizontal([Constraint::Min(1), Constraint::Length(7)]).split(area);
            f.render_widget(
                Paragraph::new(format!(" ▶ {}", status.title)).style(style(app, app.palette.text)),
                cols[0],
            );
            f.render_widget(
                Paragraph::new(time(status.position))
                    .right_aligned()
                    .style(style(app, app.palette.accent)),
                cols[1],
            );
        } else {
            f.render_widget(
                Paragraph::new(format!(
                    " {} resume · {} playback queue",
                    key(app, Action::Resume),
                    key(app, Action::PlaybackQueue)
                ))
                .style(style(app, app.palette.muted)),
                area,
            );
        }
        return;
    }
    let title = if status.paused {
        " Paused "
    } else if status.active {
        " Now playing "
    } else if !status.title.is_empty() {
        " Stopped "
    } else {
        " Player "
    };
    let mut block = panel(title, app);
    if status.active && area.width >= 75 {
        block = block.title_bottom(
            Line::from(format!(
                " shuffle {} · repeat {:?} ",
                if app.snapshot.playback.shuffle {
                    "on"
                } else {
                    "off"
                },
                app.snapshot.playback.repeat,
            ))
            .right_aligned(),
        );
    }
    let inner = inset(block.inner(area));
    f.render_widget(block, area);
    if let Some(error) = &status.error {
        let recovery = ytui_services::failure::recovery(error);
        let title = recovery.map_or("Playback failed", |r| r.title);
        let instruction = recovery.map_or(
            "Open Playback repair or Diagnostics in Actions for details.",
            |r| r.instruction,
        );
        let heading = if inner.width < 60 {
            format!("{} Actions · {title}", key(app, Action::Command))
        } else {
            format!(
                "{title} · {} Actions · {} Settings",
                key(app, Action::Command),
                key(app, Action::Settings)
            )
        };
        let mut message = vec![
            Line::styled(
                heading,
                style(app, app.palette.error).add_modifier(Modifier::BOLD),
            ),
            Line::from(instruction),
            Line::styled(error.clone(), style(app, app.palette.muted)),
        ];
        if recovery.is_none() {
            message.swap(1, 2);
        }
        f.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: true })
                .style(style(app, app.palette.text)),
            inner,
        );
    } else if status.active {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
        let right_width = if inner.width > 60 { 39 } else { 14 };
        let cols = Layout::horizontal([Constraint::Min(1), Constraint::Length(right_width)])
            .split(rows[0]);
        f.render_widget(
            Paragraph::new(format!(
                "{}  {}",
                if status.paused { "Ⅱ" } else { "▶" },
                status.title
            ))
            .style(style(app, app.palette.text).add_modifier(Modifier::BOLD)),
            cols[0],
        );
        let timing = format!(
            "{} / {}",
            time(status.position),
            status.duration.map(time).unwrap_or_else(|| "LIVE".into())
        );
        f.render_widget(
            Paragraph::new(if inner.width > 60 {
                format!(
                    "{timing} · {:.0}%{} · {:.2}×",
                    status.volume,
                    if status.muted { " M" } else { "" },
                    status.speed
                )
            } else {
                timing
            })
            .right_aligned()
            .style(style(app, app.palette.muted)),
            cols[1],
        );
        if let Some(duration) = status.duration.filter(|d| d.is_finite() && *d > 0.0) {
            let mut spans = Vec::new();
            for column in 0..inner.width {
                let seconds = (column as f64 + 0.5) / inner.width.max(1) as f64 * duration;
                let marked = status
                    .segments
                    .iter()
                    .any(|s| seconds >= s.start && seconds < s.end);
                spans.push(Span::styled(
                    if seconds <= status.position {
                        "█"
                    } else if marked {
                        "▒"
                    } else {
                        "░"
                    },
                    style(
                        app,
                        if marked {
                            app.palette.warning
                        } else if seconds <= status.position {
                            app.palette.accent
                        } else {
                            app.palette.muted
                        },
                    )
                    .add_modifier(Modifier::BOLD),
                ));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
            app.hit_targets.push((rows[1], HitTarget::Timeline));
        } else {
            f.render_widget(
                Paragraph::new(if status.buffering {
                    "Buffering…"
                } else {
                    "Live stream"
                })
                .style(style(app, app.palette.accent)),
                rows[1],
            );
        }
        if rows[2].height > 0 {
            let controls = [
                (Action::PreviousTrack, "[ Previous ]"),
                (
                    Action::Pause,
                    if status.paused {
                        "[ Play ]"
                    } else {
                        "[ Pause ]"
                    },
                ),
                (Action::NextTrack, "[ Next ]"),
                (
                    Action::Mute,
                    if status.muted {
                        "[ Unmute ]"
                    } else {
                        "[ Mute ]"
                    },
                ),
                (Action::PlayerMenu, "[ Controls ]"),
                (Action::SponsorUndo, "[ Replay skip ]"),
            ];
            let mut x = rows[2].x;
            for (action, label) in controls {
                let width = label.len() as u16 + 1;
                if x + width > rows[2].right() {
                    break;
                }
                let rect = Rect::new(x, rows[2].y, width, 1);
                f.render_widget(
                    Paragraph::new(label).style(style(app, app.palette.accent)),
                    rect,
                );
                app.hit_targets.push((rect, HitTarget::Action(action)));
                x += width;
            }
        }
        if rows[3].height > 0 {
            f.render_widget(
                Paragraph::new(status.notice.clone().unwrap_or_else(|| {
                    format!(
                        "SponsorBlock {} · B toggle · U replay · o all controls",
                        if status.sponsor_enabled {
                            "skip"
                        } else {
                            "marks only"
                        }
                    )
                }))
                .style(style(app, app.palette.warning)),
                rows[3],
            );
        }
    } else {
        f.render_widget(
            Paragraph::new(hints(
                app,
                &[
                    (Action::Open, "play selected"),
                    (Action::Resume, "continue watching"),
                    (Action::PlaybackQueue, "queue"),
                ],
                inner.width,
            )),
            inner,
        );
    }
}
