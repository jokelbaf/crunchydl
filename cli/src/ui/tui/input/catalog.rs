use crate::ui::tui::*;

pub(crate) fn handle_search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.configure_current();
        }
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.query.push(character);
            app.schedule_search();
        }
        KeyCode::Backspace => {
            app.query.pop();
            app.schedule_search();
        }
        KeyCode::Up => select_catalog(app, -1),
        KeyCode::Down => select_catalog(app, 1),
        KeyCode::PageUp => select_catalog(app, -8),
        KeyCode::PageDown => select_catalog(app, 8),
        KeyCode::Enter => app.open_current(),
        _ => {}
    }
}

pub(crate) fn handle_browse_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            if let Some((items, selected)) = app.browse_parents.pop() {
                app.items = items;
                app.selected = selected;
                app.screen = if app.browse_parents.is_empty() {
                    Screen::Search
                } else {
                    Screen::Browse
                };
                app.request_thumbnail();
            } else {
                app.screen = Screen::Search;
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.configure_current();
        }
        KeyCode::Up => select_catalog(app, -1),
        KeyCode::Down => select_catalog(app, 1),
        KeyCode::PageUp => select_catalog(app, -8),
        KeyCode::PageDown => select_catalog(app, 8),
        KeyCode::Enter => app.open_current(),
        _ => {}
    }
}

pub(crate) fn select_catalog(app: &mut App, delta: isize) {
    app.selected = move_index(app.selected, app.items.len(), delta);
    app.request_thumbnail();
}

pub(crate) fn handle_selection_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.screen = app.previous_screen,
        KeyCode::Char('a') | KeyCode::Right => {
            let count = app.selection.audio_choices().len() + 2;
            app.selection.audio_index = (app.selection.audio_index + 1) % count.max(1);
        }
        KeyCode::Char('s') => {
            let count = app.selection.subtitle_choices().len() + 2;
            app.selection.subtitle_index = (app.selection.subtitle_index + 1) % count.max(1);
        }
        KeyCode::Char('q') => {
            let count = app.selection.quality_choices().len() + 1;
            app.selection.quality_index = (app.selection.quality_index + 1) % count.max(1);
        }
        KeyCode::Char('f') => {
            app.selection.format = match app.selection.format {
                QueueFormat::Matroska => QueueFormat::Mp4,
                QueueFormat::Mp4 => QueueFormat::Matroska,
            };
            if app.selection.format == QueueFormat::Mp4 {
                app.selection.subtitle_index = 1;
                app.selection.chapters = false;
            }
        }
        KeyCode::Char('c') if app.selection.format == QueueFormat::Matroska => {
            app.selection.chapters = !app.selection.chapters;
        }
        KeyCode::Char('i') if app.selection.is_collection() => {
            app.selection.include_specials = !app.selection.include_specials;
        }
        KeyCode::Char('o') => app.selection.replace = !app.selection.replace,
        KeyCode::Enter => app.add_selection_to_queue()?,
        _ => {}
    }
    Ok(())
}
