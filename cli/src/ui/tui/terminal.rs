use super::*;

pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
pub(crate) const ACCENT: Color = Color::Rgb(92, 200, 255);
pub(crate) const SURFACE: Color = Color::Rgb(35, 39, 52);
pub(crate) const MUTED: Color = Color::Rgb(130, 138, 160);
pub(crate) const SUCCESS: Color = Color::Rgb(100, 210, 140);
pub(crate) const WARNING: Color = Color::Rgb(245, 190, 80);
pub(crate) const DANGER: Color = Color::Rgb(245, 105, 120);

pub(crate) type Backend = CrosstermBackend<io::Stdout>;

pub(crate) struct TerminalSession {
    pub(crate) terminal: Terminal<Backend>,
}

impl TerminalSession {
    pub(crate) fn enter() -> Result<Self> {
        enable_raw_mode().map_err(|_| Error::TerminalInput)?;
        let mut stdout = io::stdout();
        if execute!(stdout, EnterAlternateScreen).is_err() {
            let _ = disable_raw_mode();
            return Err(Error::TerminalInput);
        }
        let terminal =
            Terminal::new(CrosstermBackend::new(stdout)).map_err(|_| Error::TerminalInput)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
