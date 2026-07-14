use crate::ui::tui::*;

pub(crate) fn draw_settings(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let content = centered(area, 96, 28);
    let items = SettingsField::ALL.into_iter().map(|field| {
        let display = setting_value(app, field);
        ListItem::new(Line::from(vec![
            Span::styled(format!("{:<22}", field.label()), Style::default().fg(MUTED)),
            Span::styled(display, Style::default().fg(Color::White)),
        ]))
    });
    let list = List::new(items)
        .block(panel(" Settings • Enter to edit "))
        .highlight_symbol("› ")
        .highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(Some(app.settings_selected));
    frame.render_stateful_widget(list, content, &mut state);
    if let Some(field) = app.settings_editing {
        let popup = centered(content, 76, 7);
        frame.render_widget(Clear, popup);
        let displayed = if matches!(field, SettingsField::LicenseEndpoint) {
            "•".repeat(app.edit_buffer.chars().count())
        } else {
            app.edit_buffer.clone()
        };
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::styled(
                    field.label(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    if displayed.is_empty() {
                        "Type a new value...".to_string()
                    } else {
                        displayed
                    },
                    Style::default().fg(Color::White),
                ),
            ]))
            .block(panel(" Edit • Enter save • Esc cancel ")),
            popup,
        );
    }
}

pub(crate) fn setting_value(app: &App, field: SettingsField) -> String {
    match field {
        SettingsField::OutputDirectory => app.config.output_dir.display().to_string(),
        SettingsField::Filename => app.config.filename.clone(),
        SettingsField::FolderLayout => app
            .config
            .output_layout
            .clone()
            .unwrap_or_else(|| "Disabled".to_string()),
        SettingsField::DrmBackend => app.config.drm_backend.to_string(),
        SettingsField::DrmDevice => app.config.drm_device.as_deref().map_or_else(
            || "Not configured".to_string(),
            |path| path.display().to_string(),
        ),
        SettingsField::LicenseEndpoint => {
            if app.config.license_endpoint.is_some() {
                "Custom override".to_string()
            } else {
                "Automatic".to_string()
            }
        }
    }
}

pub(crate) fn draw_account(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let content = centered(area, 72, 20);
    let lines = vec![
        Line::styled(
            "Crunchyroll account",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![label("Profile"), value(&app.account.name)]),
        Line::from(vec![
            label("Email"),
            value(app.account.email.as_deref().unwrap_or("Not available")),
        ]),
        Line::from(vec![label("Premium"), bool_value(app.account.premium)]),
        Line::raw(""),
        Line::styled(
            "Application data",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::from(vec![
            label("Config"),
            value(&app.paths.config.display().to_string()),
        ]),
        Line::from(vec![
            label("Queue"),
            value(&app.paths.queue.display().to_string()),
        ]),
        Line::from(vec![
            label("Archive"),
            value(&app.paths.archive.display().to_string()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                " L ",
                Style::default()
                    .fg(Color::Black)
                    .bg(DANGER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Sign out", Style::default().fg(Color::White)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Account ")),
        content,
    );
}

pub(crate) fn draw_help(frame: &mut ratatui::Frame<'_>, area: Rect) {
    let content = centered(area, 94, 30);
    let lines = vec![
        Line::styled(
            "Keyboard shortcuts",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        help_line("F1", "Discover and browse the catalog"),
        help_line("F2", "Open the scrollable download queue"),
        help_line("F3", "Edit output and DRM settings"),
        help_line("F4", "View account details or sign out"),
        help_line("F5 / ?", "Open this help screen"),
        Line::raw(""),
        Line::styled(
            "Catalog",
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        ),
        help_line("Enter", "Open a collection or configure a playable item"),
        help_line("Ctrl-D", "Configure the selected item or entire collection"),
        help_line("↑ ↓ PgUp PgDn", "Navigate long result lists"),
        Line::raw(""),
        Line::styled(
            "Queue",
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        ),
        help_line("S", "Start all pending downloads"),
        help_line("R / Shift-R", "Retry selected / all failed downloads"),
        help_line("D / Delete", "Remove the selected queue item"),
        help_line("C", "Remove all completed entries"),
        help_line("X / Ctrl-C", "Cancel the active download"),
        Line::raw(""),
        help_line("Esc", "Go back"),
        help_line("Ctrl-C", "Exit when no download is active"),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(panel(" Help ")),
        content,
    );
}

pub(crate) fn help_line(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<16}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description.to_string(), Style::default().fg(MUTED)),
    ])
}
