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
fn panel(title: impl Into<Line<'static>>, app: &App) -> Block<'static> {
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
fn modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else {
        return;
    };
    let size = match modal {
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
        Modal::Text { .. } => (90, 28),
    };
    let rect = centered(area, size.0, size.1);
    f.render_widget(Clear, rect);
    match modal {
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
                        Line::styled(*label, Style::default().add_modifier(Modifier::BOLD)),
                        Line::styled(
                            *description,
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
                    "Make yourself at home",
                    Style::default()
                        .fg(themed(app, app.palette.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Line::from("↑/↓ or PgUp/PgDn scroll · Esc closes · :keys edits shortcuts"),
                Line::from(""),
            ];
            for (_, action, _, description) in REGISTRY {
                let mapped = workspace::key(app, *action);
                lines.push(Line::from(format!("{mapped:>12}   {description}")));
            }
            lines.extend([
                Line::from(""),
                Line::from(":home :subscriptions :later :liked :library :history"),
                Line::from(":radio name :country DE :tag jazz :favorites :cliamp"),
                Line::from(":auth :settings :details :songs QUERY :albums :artists"),
                Line::from(":downloads completed QUERY :stats :groups :download-page 1,3:7"),
                Line::from(":schedule 30m URL :clipboard :profiles :profile load NAME"),
                Line::from(":offline audio QUERY · :resume · :watch add 30m · :watches · :log"),
            ]);
            f.render_widget(
                Paragraph::new(lines)
                    .scroll((*offset as u16, 0))
                    .block(panel(" Keyboard & commands ", app))
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
            f.render_widget(Paragraph::new(format!(" {} complete · {} failed · {} audio · {} output paths · {:.1}h completed media\n {:.1} MiB recorded transfer bytes · search: {}", stats.completed, stats.failed, stats.audio, stats.outputs, stats.media_seconds as f64 / 3600.0, stats.recorded_bytes as f64 / 1048576.0, if view.query.is_empty() { "—" } else { &view.query })).style(Style::default().fg(themed(app, app.palette.muted))), chunks[0]);
            let items = rows
                .iter()
                .map(|(_, job)| {
                    ListItem::new(vec![
                        Line::from(format!(
                            "{:?} · {}{}",
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
            f.render_widget(Paragraph::new(" ↑/↓ navigate · f filter · Enter shows queue item · g expands group\n :downloads completed QUERY · :stats failed · Esc closes").style(Style::default().fg(themed(app, app.palette.accent))), chunks[2]);
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
                f.render_widget(Paragraph::new("\n  No collection watches yet.\n  Open a playlist, then run :watch add 30m.")
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
            let block = panel(format!(" {} ", form.title()), app).title_bottom(
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
                        field.label,
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
                        "● on".into()
                    } else {
                        "○ off".into()
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
                Paragraph::new(message)
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
            } else {
                "Playlist indices: 1,3:7 · Quality: best or height (e.g. 1080)"
            };
            f.render_widget(
                Paragraph::new(hint).style(Style::default().fg(themed(app, app.palette.muted))),
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
