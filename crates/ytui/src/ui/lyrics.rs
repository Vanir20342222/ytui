use super::*;
use crate::lyrics::View;

pub(super) fn render(f: &mut Frame, app: &App, view: &View, rect: Rect) {
    let block = panel(
        format!(" {} · {} ", app.tr("Lyrics"), app.tr(view.source)),
        app,
    )
    .title_bottom(Line::from(
        app.tr("↑/↓ scroll · f follow · Backspace candidates · Esc close"),
    ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if view.loading {
        f.render_widget(Paragraph::new(app.tr("Loading lyrics…")), inner);
        return;
    }
    if let Some(error) = &view.error {
        f.render_widget(
            Paragraph::new(error.as_str()).wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    if view.tracks.is_empty() {
        f.render_widget(
            Paragraph::new(app.tr(
                "No lyrics found. Edit the title/artist and try again, or open a local LRC file.",
            ))
            .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    if !view.opened {
        let mut state = ListState::default().with_selected(Some(view.selected));
        let items = view
            .tracks
            .iter()
            .map(|track| {
                ListItem::new(vec![
                    Line::from(format!("{} — {}", track.title, track.artist)),
                    Line::styled(
                        format!(
                            "{} · {} · {}",
                            track.album,
                            time(track.duration),
                            app.tr(if track.instrumental {
                                "Instrumental"
                            } else if track.synced() {
                                "Timed lyrics"
                            } else {
                                "Plain lyrics"
                            })
                        ),
                        Style::default().fg(themed(app, app.palette.muted)),
                    ),
                    Line::from(""),
                ])
            })
            .collect::<Vec<_>>();
        f.render_stateful_widget(
            List::new(items).highlight_symbol("› ").highlight_style(
                Style::default()
                    .fg(themed(app, app.palette.accent))
                    .bg(themed(app, app.palette.selection)),
            ),
            inner,
            &mut state,
        );
        f.render_widget(
            Paragraph::new(app.tr("Enter opens the selected match")),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
        return;
    }
    let Some(track) = view.tracks.get(view.selected) else {
        return;
    };
    let matches_player = app
        .player_status
        .media
        .as_ref()
        .is_some_and(|media| Some(media.url.as_str()) == view.media_url.as_deref());
    let active = matches_player
        .then(|| track.active_line(app.player_status.position))
        .flatten();
    let areas = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(inner);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(format!("{} — {}", track.title, track.artist)),
            Line::styled(
                app.tr(if !matches_player {
                    "Timing paused: these lyrics belong to a different selection."
                } else if view.follow && track.synced() {
                    "Following playback · ↑/↓ to scroll manually"
                } else {
                    "Manual scroll · f follows playback"
                }),
                Style::default().fg(themed(app, app.palette.muted)),
            ),
        ])
        .wrap(Wrap { trim: true }),
        areas[0],
    );
    if track.instrumental {
        f.render_widget(
            Paragraph::new(app.tr("Instrumental — no sung lyrics")),
            areas[1],
        );
        return;
    }
    let visible = areas[1].height as usize;
    let start = if view.follow {
        active.unwrap_or(0).saturating_sub(visible / 2)
    } else {
        view.offset
    };
    // One terminal row per source line keeps scrolling/timestamp alignment stable.
    for (row, (index, line)) in track
        .lines
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let selected = active == Some(index);
        f.render_widget(
            Paragraph::new(format!(
                "{} {}",
                if selected { "›" } else { " " },
                line.text
            ))
            .style(
                Style::default()
                    .fg(themed(
                        app,
                        if selected {
                            app.palette.accent
                        } else {
                            app.palette.text
                        },
                    ))
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Rect::new(areas[1].x, areas[1].y + row as u16, areas[1].width, 1),
        );
    }
}
