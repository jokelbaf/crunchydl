use crate::ui::tui::*;

pub(crate) fn handle_queue_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.show(Screen::Search),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Up => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), -1);
        }
        KeyCode::Down => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), 1);
        }
        KeyCode::PageUp => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), -8);
        }
        KeyCode::PageDown => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), 8);
        }
        KeyCode::Home => app.queue_selected = 0,
        KeyCode::End => app.queue_selected = app.queue_items.len().saturating_sub(1),
        KeyCode::Char('s') if !app.queue_running => app.start_queue(),
        KeyCode::Char('x') if app.queue_running => {
            if let Some(cancellation) = &app.queue_cancellation {
                cancellation.cancel();
                app.set_notice(NoticeKind::Warning, "Cancelling active download...");
            }
        }
        KeyCode::Char('r') if !app.queue_running => app.retry_selected()?,
        KeyCode::Char('R') if !app.queue_running => app.retry_all()?,
        KeyCode::Char('c') if !app.queue_running => {
            app.confirmation = Some(Confirmation::ClearCompleted);
        }
        KeyCode::Delete | KeyCode::Char('d') if !app.queue_running => app.remove_selected(),
        _ => {}
    }
    Ok(())
}

pub(crate) fn handle_settings_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.show(Screen::Search),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Up => {
            app.settings_selected = move_index(app.settings_selected, SettingsField::ALL.len(), -1);
        }
        KeyCode::Down => {
            app.settings_selected = move_index(app.settings_selected, SettingsField::ALL.len(), 1);
        }
        KeyCode::Left | KeyCode::Right
            if matches!(
                SettingsField::ALL[app.settings_selected],
                SettingsField::DrmBackend
            ) =>
        {
            app.begin_setting_edit();
        }
        KeyCode::Enter => app.begin_setting_edit(),
        _ => {}
    }
}

pub(crate) fn handle_account_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.show(Screen::Search),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('l') => app.confirmation = Some(Confirmation::Logout),
        _ => {}
    }
}

pub(crate) fn handle_help_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.screen = app.previous_screen,
        KeyCode::Char('q') => app.should_quit = true,
        _ => {}
    }
}

pub(crate) fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize).min(len - 1)
    }
}
