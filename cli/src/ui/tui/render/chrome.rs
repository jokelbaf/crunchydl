use crate::ui::tui::*;

pub(crate) fn draw(frame: &mut ratatui::Frame<'_>, app: &mut App) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(20, 23, 32))),
        area,
    );
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    draw_navigation(frame, app, layout[0]);
    match app.screen {
        Screen::Search | Screen::Browse => draw_catalog(frame, app, layout[1]),
        Screen::Selection => draw_selection(frame, app, layout[1]),
        Screen::Queue => draw_queue(frame, app, layout[1]),
        Screen::Settings => draw_settings(frame, app, layout[1]),
        Screen::Account => draw_account(frame, app, layout[1]),
        Screen::Help => draw_help(frame, layout[1]),
    }
    draw_footer(frame, app, layout[2]);
    if let Some(confirmation) = &app.confirmation {
        draw_confirmation(frame, confirmation);
    }
}

pub(crate) fn draw_navigation(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let queue_count = app.queue_items.len();
    let queue_title = format!("Queue {queue_count}");
    let compact = area.width < 100;
    let left = vec![
        Span::styled(
            " crunchydl ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        nav_span(
            "F1",
            if compact { "Home" } else { "Discover" },
            matches!(app.screen, Screen::Search | Screen::Browse),
        ),
        Span::raw("  "),
        nav_span("F2", &queue_title, app.screen == Screen::Queue),
        Span::raw("  "),
        nav_span(
            "F3",
            if compact { "Setup" } else { "Settings" },
            app.screen == Screen::Settings,
        ),
        Span::raw("  "),
        nav_span(
            "F4",
            if compact { "Me" } else { "Account" },
            app.screen == Screen::Account,
        ),
        Span::raw("  "),
        nav_span(
            "F5",
            if compact { "?" } else { "Help" },
            app.screen == Screen::Help,
        ),
    ];
    let premium = if app.account.premium {
        "PREMIUM"
    } else {
        "FREE"
    };
    let right = if compact {
        format!("{premium} ")
    } else {
        format!("{}  {premium} ", ellipsize(&app.account.name, 24))
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(40),
            Constraint::Length(right.chars().count() as u16),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(left)).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(SURFACE)),
        ),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(if app.account.premium { WARNING } else { MUTED }))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(SURFACE)),
            ),
        columns[1],
    );
}

pub(crate) fn nav_span<'a>(key: &'a str, label: &'a str, active: bool) -> Span<'a> {
    if active {
        Span::styled(
            format!(" {key} {label} "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!("{key} {label}"), Style::default().fg(MUTED))
    }
}

pub(crate) fn draw_footer(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let color = match app.notice.kind {
        NoticeKind::Info => ACCENT,
        NoticeKind::Success => SUCCESS,
        NoticeKind::Warning => WARNING,
        NoticeKind::Error => DANGER,
    };
    let icon = match app.notice.kind {
        NoticeKind::Info => "●",
        NoticeKind::Success => "✓",
        NoticeKind::Warning => "!",
        NoticeKind::Error => "×",
    };
    let help = footer_help(app.screen, app.queue_running, area.width < 100);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                ellipsize(
                    &app.notice.text,
                    columns[0].width.saturating_sub(5) as usize,
                ),
                Style::default().fg(color),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(SURFACE)),
        ),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(help)
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(SURFACE)),
            ),
        columns[1],
    );
}

pub(crate) fn footer_help(screen: Screen, running: bool, compact: bool) -> &'static str {
    if compact {
        return match screen {
            Screen::Search | Screen::Browse => "↑↓ Move  Enter Open  Ctrl-D Add ",
            Screen::Selection => "A/S/Q Choose  Enter Queue ",
            Screen::Queue if running => "↑↓ Scroll  X Cancel ",
            Screen::Queue => "↑↓ Scroll  S Start  R Retry  D Delete ",
            Screen::Settings => "↑↓ Move  Enter Edit ",
            Screen::Account => "L Sign out ",
            Screen::Help => "Esc Back ",
        };
    }
    match screen {
        Screen::Search | Screen::Browse => "↑↓ Navigate  Enter Open  Ctrl-D Download ",
        Screen::Selection => "A Audio  S Subtitles  Q Quality  Enter Queue ",
        Screen::Queue if running => "↑↓ Scroll  X Cancel  Ctrl-C Cancel ",
        Screen::Queue => "↑↓ Scroll  S Start  R Retry  D Delete  C Clear ",
        Screen::Settings => "↑↓ Navigate  Enter Edit  Esc Back ",
        Screen::Account => "L Sign out  Esc Back ",
        Screen::Help => "Esc Back  Ctrl-C Quit ",
    }
}

pub(crate) fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE))
        .padding(Padding::horizontal(1))
}
