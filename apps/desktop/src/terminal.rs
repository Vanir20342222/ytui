use anyhow::Result;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{self, Write};
pub type Term = Terminal<CrosstermBackend<io::Stdout>>;
pub struct Guard;
impl Guard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(e) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        ) {
            let _ = disable_raw_mode();
            return Err(e.into());
        }
        Ok(Self)
    }
    pub fn suspend() -> Result<()> {
        disable_raw_mode()?;
        execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        )?;
        Ok(())
    }
    pub fn resume() -> Result<()> {
        enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = Self::suspend();
        let _ = io::stdout().flush();
    }
}
