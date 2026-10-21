use super::*;
use crate::{app::HitTarget, editor::Editor};
pub(super) fn actions(
    f: &mut Frame,
    app: &App,
    selected: usize,
    query: &Editor,
    rect: Rect,
) -> Vec<(Rect, HitTarget)> {
    let mut hits = Vec::new();
    let block = panel(" Actions · type to filter ", app)
        .title_bottom(" ↑/↓ choose · Enter open · Esc close ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(inner);
    f.render_widget(
        Paragraph::new(format!(" › {}", query.text))
            .style(Style::default().fg(themed(app, app.palette.accent))),
        rows[0],
    );
    let matches = crate::actions::matching(&query.text);
    let visible = (rows[1].height / 2).max(1) as usize;
    let start = selected.saturating_sub(visible - 1);
    for (position, index) in matches.iter().enumerate().skip(start).take(visible) {
        let entry = &crate::actions::ENTRIES[*index];
        let row = Rect::new(
            rows[1].x + 1,
            rows[1].y + (position - start) as u16 * 2,
            rows[1].width.saturating_sub(2),
            2.min(rows[1].height),
        );
        let style = Style::default()
            .fg(themed(
                app,
                if position == selected {
                    app.palette.accent
                } else {
                    app.palette.text
                },
            ))
            .bg(themed(
                app,
                if position == selected {
                    app.palette.selection
                } else {
                    app.palette.background
                },
            ));
        f.render_widget(
            Paragraph::new(vec![
                Line::from(format!(" {}", entry.title)),
                Line::styled(
                    format!(" {}", entry.group),
                    Style::default().fg(themed(app, app.palette.muted)),
                ),
            ])
            .style(style),
            row,
        );
        hits.push((row, HitTarget::MenuItem(position)));
    }
    if matches.is_empty() {
        f.render_widget(Paragraph::new(" No matching action"), rows[1]);
    }
    hits
}
pub(super) fn categories(
    f: &mut Frame,
    app: &App,
    selected: usize,
    rect: Rect,
) -> Vec<(Rect, HitTarget)> {
    let block =
        panel(" Feed categories ", app).title_bottom(" ↑/↓ choose · Enter open · Esc close ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let categories = &app.tabs[app.tab].page.categories;
    if categories.is_empty() {
        f.render_widget(Paragraph::new("No category filters were supplied for this feed.\nOpen Home or search; available filters come from YouTube.").wrap(Wrap {trim:true}),inner);
        return Vec::new();
    }
    let mut state = ListState::default().with_selected(Some(selected));
    let list = List::new(
        categories
            .iter()
            .map(|c| ListItem::new(format!(" {}", c.title))),
    )
    .highlight_style(
        Style::default()
            .fg(themed(app, app.palette.accent))
            .bg(themed(app, app.palette.selection)),
    )
    .highlight_symbol("›");
    f.render_stateful_widget(list, inner, &mut state);
    (state.offset()..(state.offset() + inner.height as usize).min(categories.len()))
        .map(|i| {
            (
                Rect::new(
                    inner.x,
                    inner.y + (i - state.offset()) as u16,
                    inner.width,
                    1,
                ),
                HitTarget::MenuItem(i),
            )
        })
        .collect()
}
pub(super) fn search(f: &mut Frame, app: &App, rect: Rect) {
    let Some(input) = &app.input else {
        return;
    };
    f.render_widget(Clear, rect);
    let title = if app.tab == 0 {
        " Add URL or search YouTube "
    } else if app.tab == 3 {
        " Search radio "
    } else {
        " Search or paste URL "
    };
    let block = panel(title, app).title_bottom(" Enter search · Esc cancel ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let (text, cursor) = viewport(
        &input.editor.text,
        input.editor.cursor,
        inner.width.saturating_sub(2),
    );
    f.render_widget(
        Paragraph::new(format!(" {text}"))
            .style(Style::default().fg(themed(app, app.palette.accent))),
        inner,
    );
    f.set_cursor_position((inner.x + 1 + cursor, inner.y));
}
