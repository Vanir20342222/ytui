//! Level-reactive terminal art. Towers are stereo VU, not frequency bands.
use super::*;
pub(super) fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let orbit = app.config.music_player_style.starts_with("orbit");
    if !orbit && !app.config.music_player_style.starts_with("vu") {
        super::vinyl::render(f, app, area);
        return;
    }
    let name = if orbit { "Pulse Orbit" } else { "Neon VU" };
    let key = super::workspace::key(app, ytui_core::shortcuts::Action::MusicPlayerStyle);
    let block = panel(format!(" {name} · {key} view "), app);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 5 || inner.width < 12 {
        return;
    }
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(4)]).split(inner);
    let art = rows[0];
    let levels = app
        .player_status
        .audio_levels
        .and(app.visual_levels.or(app.player_status.audio_levels));
    let phase = app.vinyl_motion.phase;
    let (fw, fh) = app.picker.font_size();
    let aspect = fh.max(1) as f64 / fw.max(1) as f64;
    let height = art.height.max(1) as f64;
    let width = art.width.max(1) as f64;
    let accent = app.palette.accent;
    let secondary = app.palette.success;
    let blend = |a: (u8, u8, u8), b: (u8, u8, u8), t: f64| {
        let t = t.clamp(0.0, 1.0);
        rgb((
            (a.0 as f64 * (1.0 - t) + b.0 as f64 * t) as u8,
            (a.1 as f64 * (1.0 - t) + b.1 as f64 * t) as u8,
            (a.2 as f64 * (1.0 - t) + b.2 as f64 * t) as u8,
        ))
    };
    let pixel = |x: f64, y: f64| {
        let Some(level) = levels else {
            return themed(app, app.palette.background);
        };
        if orbit {
            let dx = (x - width / 2.0) / aspect;
            let dy = y - height / 2.0;
            let radius = (height * 0.43).min(width / aspect * 0.43).max(1.0);
            let r = (dx * dx + dy * dy).sqrt() / radius;
            let angle = dy.atan2(dx);
            let pulse = (level.left + level.right) / 2.0;
            for ring in 0..4 {
                let target = 0.22 + ring as f64 * 0.18 + pulse * 0.13;
                let wobble = (angle * 3.0 + phase + ring as f64).sin() * 0.025 * pulse;
                if (r - target - wobble).abs() < 0.025 {
                    return blend(accent, secondary, ring as f64 / 3.0);
                }
            }
            let orbit_radius = 0.5 + level.peak * 0.35;
            if ((dx / radius - orbit_radius * phase.cos()).powi(2)
                + (dy / radius - orbit_radius * phase.sin()).powi(2))
            .sqrt()
                < 0.07
            {
                return themed(app, app.palette.warning);
            }
            themed(app, app.palette.background)
        } else {
            let nx = x / width;
            let channel = if nx < 0.5 { level.left } else { level.right };
            let local = (nx * 2.0).fract();
            let bar = ((local * 8.0).fract() < 0.72) && (0.08..0.92).contains(&local);
            let normalized = 1.0 - y / height;
            if bar && normalized <= channel && (y / 1.5).fract() < 0.68 {
                if normalized > 0.88 {
                    themed(app, app.palette.error)
                } else if normalized > 0.72 {
                    themed(app, app.palette.warning)
                } else {
                    blend(secondary, accent, normalized)
                }
            } else if bar && (normalized - level.peak).abs() < 0.015 {
                themed(app, app.palette.text)
            } else {
                themed(app, app.palette.background)
            }
        }
    };
    for y in art.y..art.bottom() {
        for x in art.x..art.right() {
            let top = pixel((x - art.x) as f64 + 0.5, (y - art.y) as f64 + 0.25);
            let bottom = pixel((x - art.x) as f64 + 0.5, (y - art.y) as f64 + 0.75);
            f.buffer_mut()[(x, y)]
                .set_symbol("▀")
                .set_fg(top)
                .set_bg(bottom);
        }
    }
    if levels.is_none() {
        let message = if !app.player_status.active {
            "Play a song or station"
        } else {
            "Audio meter unavailable · playback continues"
        };
        f.render_widget(
            Paragraph::new(app.tr(message))
                .centered()
                .style(Style::default().fg(themed(app, app.palette.muted))),
            Rect::new(art.x, art.y + art.height / 2, art.width, 1),
        );
    }
    let title = app
        .player_status
        .media
        .as_ref()
        .map_or("Choose a song or station", |m| m.title.as_str());
    let creator = app
        .player_status
        .media
        .as_ref()
        .map_or("", |m| m.creator.as_str());
    let state = if app.player_status.paused {
        "Paused"
    } else if app.player_status.buffering {
        "Buffering"
    } else if app.player_status.active {
        "Playing"
    } else {
        "Stopped"
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                title,
                Style::default()
                    .fg(themed(app, app.palette.text))
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(creator, Style::default().fg(themed(app, accent))),
            Line::from(if orbit {
                format!("{state} · stream RMS / peak")
            } else {
                format!("L  ·  {state} · stereo dBFS  ·  R")
            }),
            Line::from(format!(
                "{} pause · {} controls · Esc leaves fullscreen",
                super::workspace::key(app, ytui_core::shortcuts::Action::Pause),
                super::workspace::key(app, ytui_core::shortcuts::Action::PlayerMenu)
            )),
        ])
        .centered(),
        rows[1],
    );
}
