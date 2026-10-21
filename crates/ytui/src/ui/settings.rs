use super::*;
use crate::{app::HitTarget, forms::Form};

pub(super) fn render(f: &mut Frame, app: &App, form: &Form, rect: Rect) -> Vec<(Rect, HitTarget)> {
    let mut hits = Vec::new();
    f.render_widget(Clear, rect);
    let block = panel(format!(" {} ", form.title()), app);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(4),
    ])
    .split(inner);
    let (title, indices) = form.sections()[form.section];
    f.render_widget(
        Paragraph::new(format!(
            " {title}  ·  {} / {}",
            form.section + 1,
            form.sections().len()
        ))
        .style(
            Style::default()
                .fg(themed(app, app.palette.accent))
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
    );
    let wide = rect.width >= 80 && rect.height >= 18;
    let cols = Layout::horizontal([
        Constraint::Length(if wide { 24 } else { 0 }),
        Constraint::Min(1),
    ])
    .split(chunks[1]);
    if wide {
        for (index, (name, _)) in form.sections().iter().enumerate() {
            let row = Rect::new(
                cols[0].x + 1,
                cols[0].y + index as u16 * 2,
                cols[0].width.saturating_sub(2),
                1,
            );
            if row.y >= cols[0].bottom() {
                break;
            }
            let active = index == form.section;
            f.render_widget(
                Paragraph::new(format!("{} {name}", if active { "›" } else { " " })).style(
                    Style::default()
                        .fg(themed(
                            app,
                            if active {
                                app.palette.accent
                            } else {
                                app.palette.muted
                            },
                        ))
                        .add_modifier(if active && form.sections_focused {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                row,
            );
            hits.push((row, HitTarget::SettingSection(index)));
        }
    }
    let selected = indices
        .iter()
        .position(|i| *i == form.selected)
        .unwrap_or(0);
    let row_height = if wide { 2 } else { 1 };
    let visible = (cols[1].height / row_height).max(1) as usize;
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for (position, index) in indices.iter().enumerate().skip(start).take(visible) {
        let field = &form.fields[*index];
        let row = Rect::new(
            cols[1].x + 1,
            cols[1].y + (position - start) as u16 * row_height,
            cols[1].width.saturating_sub(2),
            1,
        );
        let active = *index == form.selected && !form.sections_focused;
        let style = Style::default()
            .fg(themed(
                app,
                if active {
                    app.palette.accent
                } else {
                    app.palette.text
                },
            ))
            .bg(themed(
                app,
                if active {
                    app.palette.selection
                } else {
                    app.palette.background
                },
            ));
        let label_width = if row.width > 60 { 31 } else { row.width / 2 };
        f.render_widget(
            Paragraph::new(field.label).style(style),
            Rect::new(row.x, row.y, label_width.saturating_sub(1), 1),
        );
        let value_area = Rect::new(
            row.x + label_width,
            row.y,
            row.width.saturating_sub(label_width),
            1,
        );
        let (text, cursor) = viewport(
            &field.value.text,
            field.value.cursor,
            value_area.width.saturating_sub(2),
        );
        let text = if field.toggle {
            if field.value.text == "yes" {
                "● on".into()
            } else {
                "○ off".into()
            }
        } else if !field.choices.is_empty() {
            format!("‹ {text} ›")
        } else {
            text
        };
        f.render_widget(Paragraph::new(text).style(style), value_area);
        if active && !field.toggle && field.choices.is_empty() && value_area.width > 0 {
            f.set_cursor_position((
                value_area.x + cursor.min(value_area.width - 1),
                value_area.y,
            ));
        }
        hits.push((row, HitTarget::SettingField(*index)));
    }
    let footer = chunks[2];
    let download = matches!(form.kind, crate::forms::FormKind::Download(..));
    let default_message = if download {
        match form.section {
            0 => "Choose audio/video quality and format. Enter starts the download.",
            1 => "Choose a destination; playlist indices use ranges such as 1,3:7.",
            _ => "Subtitle languages are comma-separated; captions depend on the source.",
        }
    } else {
        match form.section {
            0 => "Changes stay in this window until saved. Esc discards edits.",
            1 => "System follows your desktop palette. Left/Right cycles choices.",
            2 => "Defaults apply to new jobs. Rates are KiB/s; 0 disables a cap.",
            3 => "Languages and SponsorBlock categories are comma-separated.",
            4 => "F2 → Account connects your browser. Credentials stay out of settings.",
            5 => "Local time HH:MM-HH:MM; blank disables a window.",
            6 => "External tools are launched only by their corresponding actions.",
            8 => "Skip marks sponsors and skips them. B toggles; U replays a skipped part.",
            _ => "Countries: DE,US. Servers: comma-separated HTTP(S) origins; blank discovers.",
        }
    };
    let message = form.error.as_deref().unwrap_or(default_message);
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
        Rect::new(footer.x + 1, footer.y, footer.width.saturating_sub(2), 2),
    );
    let buttons = Layout::horizontal([
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Min(0),
    ])
    .split(Rect::new(
        footer.x + 1,
        footer.y + 2,
        footer.width.saturating_sub(2),
        1,
    ));
    f.render_widget(
        Paragraph::new(if download {
            "[Enter Start]"
        } else {
            "[Enter Save]"
        })
        .style(Style::default().fg(themed(app, app.palette.accent))),
        buttons[0],
    );
    f.render_widget(
        Paragraph::new("[Esc Cancel]").style(Style::default().fg(themed(app, app.palette.muted))),
        buttons[1],
    );
    hits.push((buttons[0], HitTarget::SettingsSave));
    hits.push((buttons[1], HitTarget::SettingsCancel));
    f.render_widget(
        Paragraph::new("PgUp/PgDn sections · F6 focus · ↑/↓ fields · Space toggle")
            .style(Style::default().fg(themed(app, app.palette.muted))),
        Rect::new(
            footer.x + 1,
            footer.y + 3,
            footer.width.saturating_sub(2),
            1,
        ),
    );
    hits
}
