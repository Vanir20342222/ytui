use crate::app::{App, Modal, TAB_NAMES};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
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
pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(
        Block::default().style(
            Style::default()
                .bg(themed(app, app.palette.background))
                .fg(themed(app, app.palette.text)),
        ),
        area,
    );
    if area.width < 35 || area.height < 12 {
        f.render_widget(
            Paragraph::new(
                "ytui\nPlease enlarge the terminal to at least 35 × 12.\nCtrl+C quits safely.",
            )
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let editing = app.input.is_some();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(if editing { 3 } else { 2 }),
        ])
        .split(area);
    let account = app
        .session
        .as_ref()
        .map(|s| s.account.as_str())
        .unwrap_or("Anonymous");
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " ytui ",
                Style::default()
                    .fg(themed(app, app.palette.accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / ", Style::default().fg(themed(app, app.palette.muted))),
            Span::raw(if app.demo {
                "Preview your next favorite corner."
            } else {
                "Watch. Listen. Keep."
            }),
        ]))
        .style(Style::default().bg(themed(app, app.palette.background))),
        chunks[0],
    );
    let account_width = account.width().min(28) as u16;
    if chunks[0].width > 70 {
        f.render_widget(
            Paragraph::new(account).style(Style::default().fg(themed(app, app.palette.muted))),
            Rect::new(
                chunks[0].right().saturating_sub(account_width + 2),
                chunks[0].y,
                account_width,
                1,
            ),
        );
    }
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(23),
            Constraint::Percentage(23),
            Constraint::Percentage(23),
            Constraint::Percentage(31),
        ])
        .split(chunks[1]);
    for (i, name) in TAB_NAMES.iter().enumerate() {
        let selected = i == app.tab;
        f.render_widget(
            Paragraph::new(format!("{}  {name}", i + 1))
                .centered()
                .block(
                    Block::default()
                        .borders(Borders::BOTTOM)
                        .border_style(Style::default().fg(themed(
                            app,
                            if selected {
                                app.palette.accent
                            } else {
                                app.palette.surface
                            },
                        ))),
                )
                .style(
                    Style::default()
                        .fg(themed(
                            app,
                            if selected {
                                app.palette.accent
                            } else {
                                app.palette.muted
                            },
                        ))
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            tabs[i],
        );
    }
    if app.tab == 0 {
        downloads(f, app, chunks[2]);
    } else {
        browse(f, app, chunks[2]);
    }
    playback(f, app, chunks[3]);
    if let Some(input) = &app.input {
        let title = if input.command {
            " Command · Enter to run · Esc to close "
        } else if app.tab == 0 {
            " Add URL or search "
        } else {
            " Search or paste a URL "
        };
        let block = panel(title, app);
        let inner = block.inner(chunks[4]);
        let (text, cursor) = viewport(
            &input.editor.text,
            input.editor.cursor,
            inner.width.saturating_sub(1),
        );
        f.render_widget(
            Paragraph::new(text)
                .block(block)
                .style(Style::default().fg(themed(app, app.palette.accent))),
            chunks[4],
        );
        f.set_cursor_position((inner.x + cursor, inner.y));
    } else {
        let rows =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[4]);
        f.render_widget(
            Paragraph::new(format!(" {}", app.status))
                .style(Style::default().fg(themed(app, app.palette.muted))),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(if app.tab == 0 {
                " / add   Space pause/resume   d options   Del remove   r retry   ? help"
            } else {
                " / search   Enter open/play   v preview   d download   : feeds   ? help"
            })
            .style(Style::default().fg(themed(app, app.palette.accent))),
            rows[1],
        );
    }
    if app.modal.is_some() {
        modal(f, app, area);
    }
}
fn downloads(f: &mut Frame, app: &App, area: Rect) {
    let done = app
        .snapshot
        .downloads
        .jobs
        .iter()
        .filter(|j| j.state == JobState::Done)
        .count();
    let block = panel(
        format!(
            " Downloads · {} queued / {} complete ",
            app.snapshot.downloads.jobs.len() - done,
            done
        ),
        app,
    );
    if app.snapshot.downloads.jobs.is_empty() {
        f.render_widget(Paragraph::new("\n  Keep something worth coming back to.\n\n  Press / and paste a video or playlist URL,\n  or browse YouTube and press d on a video.").block(block).style(Style::default().fg(themed(app,app.palette.muted))),area);
        return;
    }
    let rows = app.snapshot.downloads.jobs.iter().map(|j| {
        let state = format!("{:?}", j.state);
        let state_color = if j.state == JobState::Error {
            app.palette.error
        } else if j.state == JobState::Done {
            app.palette.success
        } else {
            app.palette.accent
        };
        let progress = j
            .progress
            .fraction()
            .map(|p| format!("{:>3.0}%", p * 100.0))
            .unwrap_or_else(|| "  — ".into());
        let speed = j
            .progress
            .speed
            .map(|s| format!("{:.1} MiB/s", s / 1048576.0))
            .unwrap_or_default();
        let title = if j.media.title.is_empty() {
            j.media.url.clone()
        } else {
            j.media.title.clone()
        };
        let subtitle = j
            .error
            .clone()
            .or_else(|| j.outputs.last().map(|p| p.display().to_string()))
            .unwrap_or_else(|| {
                format!(
                    "{} · {} · {}",
                    if j.options.audio { "Audio" } else { "Video" },
                    j.options.quality,
                    j.options.output_directory().display()
                )
            });
        Row::new(vec![
            Cell::from(Line::styled(state, Style::default().fg(rgb(state_color)))),
            Cell::from(Text::from(vec![
                Line::from(title),
                Line::styled(
                    subtitle,
                    Style::default().fg(themed(app, app.palette.muted)),
                ),
            ])),
            Cell::from(format!("{progress}\n{speed}")),
        ])
        .height(3)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(16),
            Constraint::Length(if area.width > 75 { 13 } else { 5 }),
        ],
    )
    .block(block)
    .row_highlight_style(Style::default().bg(themed(app, app.palette.selection)))
    .highlight_symbol("▸ ")
    .column_spacing(1);
    let mut state = TableState::default().with_selected(Some(app.download_selected));
    f.render_stateful_widget(table, area, &mut state);
}
fn browse(f: &mut Frame, app: &mut App, area: Rect) {
    let view = &app.tabs[app.tab];
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(2)]).split(area);
    let title = format!(
        " {}{} · {} items{}",
        view.page.title,
        if view.loading { "  ·  Loading…" } else { "" },
        view.page.items.len(),
        if view.page.continuation.is_some() {
            "  ·  n more"
        } else {
            ""
        }
    );
    f.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(themed(app, app.palette.accent))
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
    );
    if let Some(notice) = &view.page.notice {
        f.render_widget(
            Paragraph::new(notice.as_str())
                .style(Style::default().fg(themed(app, app.palette.warning))),
            Rect::new(chunks[0].x, chunks[0].y + 1, chunks[0].width, 1),
        );
    }
    let cols = if area.width >= 105 {
        Layout::horizontal([Constraint::Percentage(61), Constraint::Percentage(39)])
            .split(chunks[1])
            .to_vec()
    } else {
        vec![chunks[1]]
    };
    let list_area = cols[0];
    if view.page.items.is_empty() {
        let message = view
            .error
            .clone()
            .or_else(|| view.page.notice.clone())
            .unwrap_or_else(|| {
                if view.loading {
                    "Finding something for you…".into()
                } else {
                    "Nothing here yet. Try / to search, :home, :library or :radio.".into()
                }
            });
        f.render_widget(
            Paragraph::new(format!("\n  {message}"))
                .wrap(Wrap { trim: true })
                .block(panel(" Browse ", app)),
            list_area,
        );
    } else {
        let column_count = if list_area.width >= 90 { 2 } else { 1 };
        let card_height = 5u16;
        let visible_rows = (list_area.height / card_height).max(1) as usize;
        let selected = view.selected;
        let first_row = (selected / column_count).saturating_sub(visible_rows - 1);
        let first = first_row * column_count;
        for (index, item) in view
            .page
            .items
            .iter()
            .enumerate()
            .skip(first)
            .take(visible_rows * column_count)
        {
            let local = index - first;
            let width = list_area.width / column_count as u16;
            let card = Rect::new(
                list_area.x + (local % column_count) as u16 * width,
                list_area.y + (local / column_count) as u16 * card_height,
                width,
                card_height,
            );
            let active = index == selected;
            let is_favorite = app.snapshot.favorites.iter().any(|m| m.url == item.url);
            let lines = vec![
                Line::styled(
                    item.title.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    item.creator.clone(),
                    Style::default().fg(themed(app, app.palette.muted)),
                ),
                Line::styled(
                    format!(
                        "{}{}{}",
                        item.label(),
                        if item.tracks_url.is_some() {
                            " · Track library"
                        } else {
                            ""
                        },
                        if is_favorite { " · ★" } else { "" }
                    ),
                    Style::default().fg(themed(app, app.palette.accent)),
                ),
            ];
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(themed(
                    app,
                    if active {
                        app.palette.accent
                    } else {
                        app.palette.surface
                    },
                )))
                .style(
                    Style::default()
                        .bg(themed(
                            app,
                            if active {
                                app.palette.selection
                            } else {
                                app.palette.background
                            },
                        ))
                        .fg(themed(app, app.palette.text)),
                );
            f.render_widget(Paragraph::new(lines).block(block), card);
        }
        if let Some(error) = &view.error {
            f.render_widget(
                Paragraph::new(error.as_str())
                    .style(Style::default().fg(themed(app, app.palette.error)))
                    .wrap(Wrap { trim: true }),
                Rect::new(
                    list_area.x,
                    list_area.bottom().saturating_sub(2),
                    list_area.width,
                    2,
                ),
            );
        }
    }
    app.image_area = Rect::default();
    if cols.len() > 1 {
        let block = panel(" Selected ", app);
        let inner = block.inner(cols[1]);
        f.render_widget(block, cols[1]);
        if let Some(media) = app.selected() {
            let image_height = (inner.height / 2).min(13);
            let image_area = Rect::new(
                inner.x + 1,
                inner.y + 1,
                inner.width.saturating_sub(2),
                image_height,
            );
            app.image_area = image_area;
            if app.config.thumbnails {
                if let Some(image) = &app.image {
                    f.render_widget(Image::new(image), image_area);
                } else {
                    f.render_widget(
                        Paragraph::new(if media.thumbnail.is_empty() {
                            if media.live {
                                "\n       ●  LIVE RADIO"
                            } else {
                                "\n       ♪  ytui"
                            }
                        } else {
                            app.image_error.as_deref().unwrap_or("Loading thumbnail…")
                        })
                        .centered()
                        .style(
                            Style::default()
                                .bg(themed(app, app.palette.surface))
                                .fg(themed(app, app.palette.muted)),
                        ),
                        image_area,
                    );
                }
            }
            let text_area = Rect::new(
                inner.x + 1,
                image_area.bottom() + 1,
                inner.width.saturating_sub(2),
                inner.bottom().saturating_sub(image_area.bottom() + 1),
            );
            let lines = vec![
                Line::styled(media.title, Style::default().add_modifier(Modifier::BOLD)),
                Line::styled(
                    media.creator,
                    Style::default().fg(themed(app, app.palette.accent)),
                ),
                Line::from(""),
                Line::from(media.description),
                Line::from(""),
                Line::styled(
                    if media.live {
                        "Enter · library / stream   :live · stream"
                    } else {
                        "Enter · open/play   v · preview"
                    },
                    Style::default().fg(themed(app, app.palette.muted)),
                ),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text_area);
        }
    }
}
fn playback(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.player_status;
    let active = !s.title.is_empty();
    let title = if s.preview {
        " Preview · muted · Esc closes "
    } else if s.paused {
        " Paused "
    } else {
        " Now playing "
    };
    let message = if let Some(error) = &s.error {
        error.clone()
    } else if active {
        format!(
            "{}  {}  {} / {}  ·  vol {:.0}%  ·  shuffle {} · repeat {:?}",
            if s.paused { "Ⅱ" } else { "▶" },
            s.title,
            time(s.position),
            s.duration
                .map(time)
                .unwrap_or_else(|| if s.buffering { "buffering" } else { "live" }.into()),
            s.volume,
            if app.snapshot.playback.shuffle {
                "on"
            } else {
                "off"
            },
            app.snapshot.playback.repeat
        )
    } else {
        "Your next watch or listen starts here.  Enter plays · e queues · p opens the queue".into()
    };
    f.render_widget(
        Paragraph::new(message)
            .block(panel(title, app))
            .style(Style::default().fg(themed(
                app,
                if s.error.is_some() {
                    app.palette.error
                } else if active {
                    app.palette.text
                } else {
                    app.palette.muted
                },
            ))),
        area,
    );
}
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
        Modal::Help => (90, 35),
        Modal::Themes(_) => (52, 20),
        Modal::Form(form) => (86, form.fields.len() as u16 + 8),
        Modal::Queue(_) => (85, 25),
    };
    let rect = centered(area, size.0, size.1);
    f.render_widget(Clear, rect);
    match modal {
        Modal::Help => {
            let mut lines = vec![
                Line::styled(
                    "Make yourself at home",
                    Style::default()
                        .fg(themed(app, app.palette.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Line::from("1–4 tabs · arrows or j/k navigate · Esc closes this help"),
                Line::from(""),
            ];
            let available = (rect.height.saturating_sub(12)) as usize;
            for (key, _, action, description) in REGISTRY.iter().take(available) {
                let mapped = app
                    .config
                    .keybindings
                    .iter()
                    .find(|(_, v)| v.as_str() == *action)
                    .map(|(k, _)| k.as_str())
                    .unwrap_or(key);
                lines.push(Line::from(format!("{mapped:>12}   {description}")));
            }
            lines.extend([
                Line::from(""),
                Line::from(":home :subscriptions :later :liked :library :history"),
                Line::from(":radio name :country DE :tag jazz :favorites :cliamp"),
                Line::from(":auth :settings :details :songs QUERY :albums :artists"),
                Line::from("All shortcuts are listed in docs/RUST.md and can be remapped."),
            ]);
            f.render_widget(
                Paragraph::new(lines)
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
            let block = panel(format!(" {} ", form.title()), app);
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
                    row,
                );
                let width = row.width.saturating_sub(label_width + 1);
                let (text, cursor) = viewport(
                    &field.value.text,
                    field.value.cursor,
                    width.saturating_sub(1),
                );
                let value_area = Rect::new(row.x + label_width, row.y, width, 1);
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
            let hint = if matches!(form.kind, crate::forms::FormKind::Auth) {
                "Only the selected profile is read. Sign in to YouTube there first."
            } else if matches!(form.kind, crate::forms::FormKind::Settings(_)) {
                ":theme lists themes · providers: auto / caelestia / omarchy / portal"
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
