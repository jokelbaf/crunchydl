use crate::ui::tui::*;

pub(crate) fn draw_queue(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(if app.queue_running { 4 } else { 0 }),
        ])
        .split(area);
    draw_queue_summary(frame, app, rows[0]);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[1]);
    let list_items = app.queue_items.iter().map(queue_list_item);
    let list = List::new(list_items)
        .block(panel(" Downloads "))
        .highlight_symbol("› ")
        .highlight_style(Style::default().bg(SURFACE));
    let mut state = ListState::default()
        .with_selected((!app.queue_items.is_empty()).then_some(app.queue_selected));
    frame.render_stateful_widget(list, columns[0], &mut state);
    draw_queue_detail(frame, app, columns[1]);
    if app.queue_running {
        let gauge = Gauge::default()
            .block(
                panel(format!(" {} ", app.progress.label)).title_style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .gauge_style(
                Style::default()
                    .fg(ACCENT)
                    .bg(SURFACE)
                    .add_modifier(Modifier::BOLD),
            )
            .ratio(app.progress.ratio())
            .label(app.progress.detail.clone());
        frame.render_widget(gauge, rows[2]);
    }
}

pub(crate) fn draw_queue_summary(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let count = |state| {
        app.queue_items
            .iter()
            .filter(|item| item.state == state)
            .count()
    };
    let line = Line::from(vec![
        Span::styled(
            " DOWNLOAD QUEUE ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        summary_badge("○", count(QueueState::Pending), "pending", MUTED),
        Span::raw("   "),
        summary_badge("●", count(QueueState::Running), "active", ACCENT),
        Span::raw("   "),
        summary_badge("✓", count(QueueState::Completed), "complete", SUCCESS),
        Span::raw("   "),
        summary_badge("×", count(QueueState::Failed), "failed", DANGER),
    ]);
    frame.render_widget(Paragraph::new(line).block(panel(" Overview ")), area);
}

pub(crate) fn summary_badge(icon: &str, count: usize, label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!("{icon} {count} {label}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn queue_list_item(item: &QueueItem) -> ListItem<'static> {
    let (icon, color) = queue_state_visual(item.state);
    let title = item.title.clone().unwrap_or_else(|| {
        item.output
            .as_deref()
            .and_then(std::path::Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .map_or_else(
                || {
                    format!(
                        "{} • {}",
                        crate::presentation::target_kind_label(&item.target),
                        item.target.id()
                    )
                },
                |name| name.to_string(),
            )
    });
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                format!("{icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                ellipsize(&title, 72),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(selection_label(item), Style::default().fg(MUTED)),
        ]),
    ])
}

pub(crate) fn queue_state_visual(state: QueueState) -> (&'static str, Color) {
    match state {
        QueueState::Pending => ("○", MUTED),
        QueueState::Running => ("●", ACCENT),
        QueueState::Completed => ("✓", SUCCESS),
        QueueState::Failed => ("×", DANGER),
    }
}

pub(crate) fn draw_queue_detail(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let Some(item) = app.current_queue() else {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::styled(
                    "Your queue is empty",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    "Find a title in Discover and press Ctrl-D to configure it.",
                    Style::default().fg(MUTED),
                ),
            ]))
            .alignment(Alignment::Center)
            .block(panel(" Details ")),
            area,
        );
        return;
    };
    let (icon, color) = queue_state_visual(item.state);
    let mut lines = vec![
        Line::from(vec![Span::styled(
            format!(" {icon} {} ", item.state),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
        Line::from(vec![
            label("Type"),
            value(crate::presentation::target_kind_label(&item.target)),
        ]),
        Line::from(vec![label("Media ID"), value(item.target.id())]),
        Line::from(vec![label("Job ID"), value(&item.id.to_string())]),
        Line::from(vec![label("Attempts"), value(&item.attempts.to_string())]),
        Line::raw(""),
        Line::styled(
            "Download choices",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::styled(selection_label(item), Style::default().fg(Color::White)),
    ];
    if let Some(output) = &item.output {
        lines.extend([
            Line::raw(""),
            Line::styled(
                "Saved to",
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                output.display().to_string(),
                Style::default().fg(Color::Gray),
            ),
        ]);
    }
    if let Some(error) = &item.failure {
        lines.extend([
            Line::raw(""),
            Line::styled(
                "What went wrong",
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
            ),
            Line::styled(safe_failure(error), Style::default().fg(Color::LightRed)),
            Line::raw(""),
            Line::styled("Press R to retry this item.", Style::default().fg(MUTED)),
        ]);
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Details ")),
        area,
    );
}
