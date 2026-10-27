//! Vinyl sleeve + rotating record, rendered entirely in the terminal.
use super::*;
use std::time::Instant;

pub struct Motion {
    pub phase: f64,
    last: Instant,
    running: bool,
}
impl Default for Motion {
    fn default() -> Self {
        Self {
            phase: 0.0,
            last: Instant::now(),
            running: false,
        }
    }
}
impl Motion {
    pub fn update(&mut self, now: Instant, running: bool) {
        if running && self.running {
            self.phase = (self.phase
                + now.saturating_duration_since(self.last).as_secs_f64() * std::f64::consts::TAU
                    / 4.0)
                % std::f64::consts::TAU;
        }
        self.last = now;
        self.running = running;
    }
}

pub(super) fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let shortcut = super::workspace::key(app, ytui_core::shortcuts::Action::MusicPlayerStyle);
    let block = panel(
        format!(" Vinyl · {shortcut} view · Esc exits fullscreen "),
        app,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 5 || inner.width < 16 {
        return;
    }
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(4)]).split(inner);
    let art = rows[0];
    let (fw, fh) = app.picker.font_size();
    let ratio = fh.max(1) as f64 / fw.max(1) as f64;
    let h = (art.height as f64)
        .min(art.width as f64 / (ratio * 1.55))
        .floor() as u16;
    let w = (h as f64 * ratio).round() as u16;
    if h > 0 && w > 0 {
        let total = (w as f64 * 1.55).ceil() as u16;
        let x = art.x + art.width.saturating_sub(total) / 2;
        let y = art.y + art.height.saturating_sub(h) / 2;
        let cx = x as f64 + w as f64 * 1.04;
        let cy = y as f64 + h as f64 / 2.0;
        let radius = h as f64 / 2.0;
        // Half blocks keep the platter round for the actual terminal font size.
        let pixel = |px: f64, py: f64| {
            let dx = (px - cx) / ratio;
            let dy = py - cy;
            let r = (dx * dx + dy * dy).sqrt() / radius;
            if r > 1.0 {
                return themed(app, app.palette.background);
            }
            let angle = dy.atan2(dx) - app.vinyl_motion.phase;
            if r < 0.28 {
                if r < 0.045 {
                    return rgb((12, 12, 14));
                }
                let (a, b, c) = app.palette.accent;
                let factor = if angle.cos() > 0.75 { 0.95 } else { 0.6 };
                return rgb((
                    (a as f64 * factor) as u8,
                    (b as f64 * factor) as u8,
                    (c as f64 * factor) as u8,
                ));
            }
            let groove = 0.82 + 0.18 * (r * std::f64::consts::TAU * 9.0).cos();
            let shine = angle.cos().powi(16) * 30.0;
            let v = ((24.0 + shine) * groove) as u8;
            rgb((v, v, v.saturating_add(3)))
        };
        for py in y..(y + h).min(art.bottom()) {
            for px in x..(x + total).min(art.right()) {
                let top = pixel(px as f64 + 0.5, py as f64 + 0.25);
                let bottom = pixel(px as f64 + 0.5, py as f64 + 0.75);
                f.buffer_mut()[(px, py)]
                    .set_symbol("▀")
                    .set_fg(top)
                    .set_bg(bottom);
            }
        }
        let sleeve = Rect::new(x, y, w.min(art.right().saturating_sub(x)), h);
        app.image_area = sleeve;
        let media = app.player_status.media.as_ref();
        if app.modal.is_none()
            && app.config.thumbnails
            && app.config.image_protocol != "none"
            && media.is_some_and(|m| app.image_matches(&m.thumbnail))
            && let Some(image) = &app.image
        {
            f.render_widget(Image::new(image), sleeve);
        } else {
            let label = media
                .map(|m| m.creator.as_str())
                .filter(|v| !v.is_empty())
                .unwrap_or("ytui");
            f.render_widget(
                Paragraph::new(format!("\n♫\n\n{label}"))
                    .centered()
                    .style(
                        Style::default()
                            .fg(themed(app, app.palette.accent))
                            .bg(themed(app, app.palette.surface)),
                    )
                    .wrap(Wrap { trim: true }),
                sleeve,
            );
        }
    }
    let status = &app.player_status;
    let state = if status.buffering {
        "Buffering"
    } else if status.paused {
        "Ⅱ Paused"
    } else if status.active {
        "▶ Playing"
    } else {
        "Stopped"
    };
    let media = status.media.as_ref();
    let title = media.map_or("Choose a song or station", |m| m.title.as_str());
    let creator = media.map_or("", |m| m.creator.as_str());
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                title,
                Style::default()
                    .fg(themed(app, app.palette.text))
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                creator,
                Style::default().fg(themed(app, app.palette.accent)),
            ),
            Line::from(state),
            Line::from(format!(
                "{} pause · {} controls · , settings",
                super::workspace::key(app, ytui_core::shortcuts::Action::Pause),
                super::workspace::key(app, ytui_core::shortcuts::Action::PlayerMenu)
            )),
        ])
        .centered(),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn rotation_freezes_and_resumes_without_catching_up_pause_time() {
        let start = Instant::now();
        let mut m = Motion {
            phase: 0.0,
            last: start,
            running: false,
        };
        m.update(start, true);
        m.update(start + Duration::from_secs(1), true);
        let phase = m.phase;
        assert!(phase > 1.5 && phase < 1.6);
        m.update(start + Duration::from_secs(2), false);
        m.update(start + Duration::from_secs(60), false);
        assert_eq!(phase, m.phase);
        m.update(start + Duration::from_secs(61), true);
        assert_eq!(phase, m.phase);
        m.update(start + Duration::from_secs(62), true);
        assert!(m.phase > phase);
    }
}
