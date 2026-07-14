use crate::ui::tui::*;

pub(crate) fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if app.confirmation.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => app.confirm()?,
            KeyCode::Char('n') | KeyCode::Esc => app.confirmation = None,
            _ => {}
        }
        return Ok(());
    }
    if app.settings_editing.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.settings_editing = None;
                app.edit_buffer.clear();
            }
            KeyCode::Enter => app.apply_setting_edit(),
            KeyCode::Backspace => {
                app.edit_buffer.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.edit_buffer.push(character);
            }
            _ => {}
        }
        return Ok(());
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if let Some(cancellation) = &app.queue_cancellation {
            cancellation.cancel();
            app.set_notice(NoticeKind::Warning, "Cancelling the active download...");
        } else {
            app.should_quit = true;
        }
        return Ok(());
    }
    match key.code {
        KeyCode::F(1) => app.show(Screen::Search),
        KeyCode::F(2) => app.show(Screen::Queue),
        KeyCode::F(3) => app.show(Screen::Settings),
        KeyCode::F(4) => app.show(Screen::Account),
        KeyCode::F(5) | KeyCode::Char('?') => app.show(Screen::Help),
        _ => match app.screen {
            Screen::Search => handle_search_key(app, key),
            Screen::Browse => handle_browse_key(app, key),
            Screen::Selection => handle_selection_key(app, key)?,
            Screen::Queue => handle_queue_key(app, key)?,
            Screen::Settings => handle_settings_key(app, key),
            Screen::Account => handle_account_key(app, key),
            Screen::Help => handle_help_key(app, key),
        },
    }
    Ok(())
}
