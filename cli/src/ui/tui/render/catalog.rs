use crate::ui::tui::*;

pub(crate) fn draw_catalog(frame: &mut ratatui::Frame<'_>, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);
    let prompt = if app.screen == Screen::Search {
        format!(" {}", app.query)
    } else {
        " Browsing collection".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "⌕",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(prompt, Style::default().fg(Color::White)),
            if app.query.is_empty() && app.screen == Screen::Search {
                Span::styled(
                    "Search anime, movies, and music...",
                    Style::default().fg(MUTED),
                )
            } else {
                Span::raw("")
            },
        ]))
        .block(panel(" Discover ")),
        rows[0],
    );
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);
    let list_items = app.items.iter().map(|item| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!(" {:<11} ", kind_label(item.kind)),
                Style::default()
                    .fg(kind_color(item.kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(item.title.clone(), Style::default().fg(Color::White)),
        ]))
    });
    let list = List::new(list_items)
        .block(panel(format!(" Results • {} ", app.items.len())))
        .highlight_symbol("› ")
        .highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(app.current().map(|_| app.selected));
    frame.render_stateful_widget(list, columns[0], &mut state);
    draw_catalog_details(frame, app, columns[1]);
}

pub(crate) fn draw_catalog_details(frame: &mut ratatui::Frame<'_>, app: &mut App, area: Rect) {
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);
    let artwork = panel(" Artwork ");
    let image_area = artwork.inner(right[0]);
    frame.render_widget(artwork, right[0]);
    if let Some((_, protocol)) = &mut app.thumbnail {
        frame.render_stateful_widget(StatefulImage::default(), image_area, protocol);
    } else {
        frame.render_widget(
            Paragraph::new(if app.thumbnail_loading.is_some() {
                "Loading artwork..."
            } else {
                "No artwork available"
            })
            .alignment(Alignment::Center)
            .style(Style::default().fg(MUTED)),
            image_area,
        );
    }
    let Some(item) = app.current() else {
        frame.render_widget(
            Paragraph::new("No item selected")
                .style(Style::default().fg(MUTED))
                .block(panel(" Details ")),
            right[1],
        );
        return;
    };
    let mut lines = vec![
        Line::styled(
            item.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![label("Type"), value(kind_label(item.kind))]),
        Line::from(vec![
            label("Rating"),
            value(
                &item
                    .rating
                    .as_ref()
                    .map_or_else(|| "Not rated".to_string(), rating_label),
            ),
        ]),
        Line::from(vec![label("Premium"), bool_value(item.premium_only)]),
        Line::from(vec![label("Subtitled"), bool_value(item.is_subbed)]),
        Line::from(vec![label("Dubbed"), bool_value(item.is_dubbed)]),
    ];
    if let Some(count) = item.episode_count {
        lines.push(Line::from(vec![
            label("Episodes"),
            value(&count.to_string()),
        ]));
    }
    lines.extend([
        Line::raw(""),
        Line::styled(
            item.extended_description
                .as_deref()
                .unwrap_or(&item.description)
                .to_string(),
            Style::default().fg(Color::Gray),
        ),
        Line::raw(""),
        Line::from(vec![
            label("ID"),
            Span::styled(item.id.clone(), Style::default().fg(MUTED)),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Details ")),
        right[1],
    );
}

pub(crate) fn label(text: &str) -> Span<'static> {
    Span::styled(format!("{text:<12}"), Style::default().fg(MUTED))
}

pub(crate) fn value(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), Style::default().fg(Color::White))
}

pub(crate) fn bool_value(value: bool) -> Span<'static> {
    Span::styled(
        yes_no(value).to_string(),
        Style::default().fg(if value { SUCCESS } else { MUTED }),
    )
}

pub(crate) fn kind_color(kind: crunchydl::CatalogKind) -> Color {
    match kind {
        crunchydl::CatalogKind::Series | crunchydl::CatalogKind::Season => ACCENT,
        crunchydl::CatalogKind::Episode => SUCCESS,
        crunchydl::CatalogKind::Movie | crunchydl::CatalogKind::MovieListing => WARNING,
        _ => Color::Magenta,
    }
}
