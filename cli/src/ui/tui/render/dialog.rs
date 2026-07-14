use crate::ui::tui::*;

pub(crate) fn draw_confirmation(frame: &mut ratatui::Frame<'_>, confirmation: &Confirmation) {
    let area = centered(frame.area(), 60, 9);
    frame.render_widget(Clear, area);
    let message = match confirmation {
        Confirmation::Remove(_) => "Remove this item from the queue?",
        Confirmation::ClearCompleted => "Remove every completed queue item?",
        Confirmation::Logout => "Sign out and remove the saved session?",
    };
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::styled(
                message,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::from(vec![
                Span::styled(
                    " Y / Enter ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(SUCCESS)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Confirm    "),
                Span::styled(" N / Esc ", Style::default().fg(Color::Black).bg(MUTED)),
                Span::raw(" Cancel"),
            ]),
        ]))
        .alignment(Alignment::Center)
        .block(panel(" Confirm action ")),
        area,
    );
}

pub(crate) fn centered(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.min(max_width);
    let height = area.height.min(max_height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
    .inner(Margin {
        horizontal: 0,
        vertical: 0,
    })
}

pub(crate) fn rating_label(rating: &crunchydl::CatalogRating) -> String {
    match rating {
        crunchydl::CatalogRating::Stars { average, total } => total.map_or_else(
            || format!("{average:.1}/5"),
            |total| format!("{average:.1}/5 • {total} ratings"),
        ),
        crunchydl::CatalogRating::Approval { percentage, total } => total.map_or_else(
            || format!("{percentage:.0}% positive"),
            |total| format!("{percentage:.0}% positive • {total} votes"),
        ),
        _ => "Not rated".to_string(),
    }
}

pub(crate) fn job_state_label(state: crunchydl::JobState) -> &'static str {
    match state {
        crunchydl::JobState::Created => "Starting",
        crunchydl::JobState::ResolvingMedia => "Resolving media",
        crunchydl::JobState::OpeningPlaybackSessions => "Opening playback",
        crunchydl::JobState::PlanningTracks => "Selecting tracks",
        crunchydl::JobState::AcquiringLicenses => "Acquiring licenses",
        crunchydl::JobState::Downloading => "Downloading",
        crunchydl::JobState::Decrypting => "Decrypting",
        crunchydl::JobState::ProcessingSubtitles => "Processing subtitles",
        crunchydl::JobState::Muxing => "Building output",
        crunchydl::JobState::Verifying => "Verifying",
        crunchydl::JobState::Committing => "Saving",
        crunchydl::JobState::Completed => "Complete",
        crunchydl::JobState::Cancelled => "Cancelled",
        crunchydl::JobState::Failed => "Failed",
        _ => "Working",
    }
}
