use crate::ui::tui::*;

pub(crate) fn draw_selection(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let content = centered(area, 84, 26);
    if app.selection.loading {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::styled(
                    "Inspecting playback options",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    "Checking audio versions, subtitles, video quality, and DRM metadata...",
                    Style::default().fg(Color::Gray),
                ),
                Line::raw(""),
                Line::styled(
                    "This may take a few seconds for titles with many dubs.",
                    Style::default().fg(MUTED),
                ),
            ]))
            .alignment(Alignment::Center)
            .block(panel(" Configure download ")),
            content,
        );
        return;
    }
    let audio = selection_audio_label(&app.selection);
    let subtitles = selection_subtitle_label(&app.selection);
    let quality = selection_quality_label(&app.selection);
    let mut lines = vec![
        Line::styled(
            app.selection.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            if app.selection.is_collection() {
                "These choices apply to every episode in the batch."
            } else {
                "Choose exactly what to include in the output."
            },
            Style::default().fg(MUTED),
        ),
        Line::raw(""),
        option_line("A", "Audio", &audio, ACCENT),
        option_line("S", "Subtitles", &subtitles, SUCCESS),
        option_line("Q", "Quality", &quality, WARNING),
        option_line(
            "F",
            "Container",
            &app.selection.format.to_string(),
            Color::Magenta,
        ),
        option_line("C", "Chapters", yes_no(app.selection.chapters), Color::Cyan),
        option_line(
            "O",
            "Replace existing",
            yes_no(app.selection.replace),
            DANGER,
        ),
    ];
    if app.selection.is_collection() {
        lines.push(option_line(
            "I",
            "Include specials",
            yes_no(app.selection.include_specials),
            Color::Yellow,
        ));
    }
    if app.selection.format == QueueFormat::Mp4 {
        lines.extend([
            Line::raw(""),
            Line::styled(
                "MP4 supports AVC/AAC only; subtitles and chapters are disabled.",
                Style::default().fg(WARNING),
            ),
        ]);
    }
    lines.extend([
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::Black)
                    .bg(SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Add to queue", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled(" Esc ", Style::default().fg(Color::Black).bg(MUTED)),
            Span::styled(" Back", Style::default().fg(Color::White)),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Configure download ")),
        content,
    );
}

pub(crate) fn option_line(key: &str, name: &str, choice: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("{name:<20}"), Style::default().fg(MUTED)),
        Span::styled(
            choice.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub(crate) fn selection_audio_label(selection: &Selection) -> String {
    match selection.audio_index {
        0 => "Original audio".to_string(),
        1 => "All available dubs".to_string(),
        index => selection.audio_choices().get(index - 2).map_or_else(
            || "Original audio".to_string(),
            |locale| format!("{} audio", locale_label_from_code(locale)),
        ),
    }
}

pub(crate) fn selection_subtitle_label(selection: &Selection) -> String {
    match selection.subtitle_index {
        0 => "All available subtitles".to_string(),
        1 => "No subtitles".to_string(),
        index => selection.subtitle_choices().get(index - 2).map_or_else(
            || "All available subtitles".to_string(),
            |locale| format!("{} subtitles", locale_label_from_code(locale)),
        ),
    }
}

pub(crate) fn selection_quality_label(selection: &Selection) -> String {
    selection
        .quality_index
        .checked_sub(1)
        .and_then(|index| selection.quality_choices().get(index).copied())
        .map_or_else(
            || "Best available".to_string(),
            |height| format!("Up to {height}p"),
        )
}
