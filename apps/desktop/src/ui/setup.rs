//! First-run setup panel (see `crate::setup`).
use super::*;
use crate::setup::{ACCOUNT, READY, Setup, TITLES, WELCOME};

pub(super) fn render(f: &mut Frame, app: &App, setup: &Setup, rect: Rect) {
    let accent = Style::default().fg(themed(app, app.palette.accent));
    let muted = Style::default().fg(themed(app, app.palette.muted));
    let text = Style::default().fg(themed(app, app.palette.text));
    let connected = app.session.is_some();
    let hint = if setup.step == READY {
        " Enter start watching · Backspace back "
    } else if setup.step == WELCOME {
        " Enter get started · Esc skip "
    } else if setup.step == ACCOUNT {
        " Space connect · Enter maybe later · Backspace back · Esc skip "
    } else {
        " ↑/↓ choose · Space/←/→ change · Enter continue · Backspace back · Esc skip "
    };
    let block = panel(format!(" ytui setup · {} ", app.tr(&setup.progress())), app)
        .title_bottom(translated(app, hint))
        .border_style(accent);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    // Step dots: filled for done, wide for the current step.
    let dots = (0..TITLES.len())
        .map(|i| {
            if i == setup.step {
                Span::styled("━━ ", accent)
            } else if i < setup.step {
                Span::styled("● ", text)
            } else {
                Span::styled("○ ", muted)
            }
        })
        .collect::<Vec<_>>();
    let mut lines = vec![
        Line::from(dots),
        Line::default(),
        Line::styled(
            app.tr(TITLES[setup.step]),
            accent.add_modifier(Modifier::BOLD),
        ),
    ];
    let body = match setup.step {
        WELCOME => {
            "YouTube, YouTube Music and radio in your terminal. A few choices and you're ready; everything stays in Settings."
        }
        crate::setup::LOOK => {
            "Pick a colour theme. It previews right away and you can change it any time in Settings."
        }
        crate::setup::PLAYBACK => "Choose how ytui plays. Every option is also in Settings later.",
        ACCOUNT => {
            "Signing in is optional. It unlocks your subscriptions, playlists and music library. ytui imports your browser session; cookie databases stay closed until you choose a profile."
        }
        _ => "Here's what you chose. Everything lives in Settings if you change your mind.",
    };
    lines.push(Line::styled(app.tr(body), text));
    lines.push(Line::default());
    for (index, row) in setup.rows(connected).iter().enumerate() {
        let selected = index == setup.row;
        let style = if selected {
            accent.add_modifier(Modifier::BOLD)
        } else {
            text
        };
        lines.push(Line::from(vec![
            Span::styled(if selected { " ▸ " } else { "   " }, style),
            Span::styled(format!("{:<34}", app.tr(row.label)), style),
            Span::styled(app.tr(&row.value).to_string(), accent),
        ]));
        lines.push(Line::styled(format!("     {}", app.tr(row.hint)), muted));
    }
    if setup.step == READY {
        for (label, value) in setup.summary(connected) {
            lines.push(Line::from(vec![
                Span::styled(format!("   {:<24}", app.tr(label)), muted),
                Span::styled(app.tr(&value).to_string(), text),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
