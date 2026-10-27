//! Isolated real-executable PTY harness. Never imports an account or plays media.
use anyhow::{Context, Result, ensure};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

pub struct Capture {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    pub parser: vt100::Parser,
    before_local_flags: Option<u32>,
}
impl Capture {
    pub fn new(binary: &Path, root: &Path, offline: bool) -> Result<Self> {
        Self::with_language(binary, root, offline, "en")
    }
    pub fn with_language(
        binary: &Path,
        root: &Path,
        offline: bool,
        language: &str,
    ) -> Result<Self> {
        ensure!(
            matches!(language, "en" | "de"),
            "Unsupported fixture language"
        );
        std::fs::create_dir_all(root)?;
        let config = root.join("config.toml");
        std::fs::write(
            &config,
            format!(
                "theme='ytui-dark'\nimage_protocol='halfblocks'\nauth_store='file'\nnotifications=false\nlanguage='{language}'\n"
            ),
        )?;
        let pair = native_pty_system().openpty(PtySize {
            rows: 58,
            cols: 212,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let before_local_flags = pair.master.get_termios().map(|t| t.local_flags.bits());
        let mut cmd = CommandBuilder::new(binary);
        cmd.args([
            "--config",
            config.to_str().context("Config path encoding")?,
            "--state-dir",
            root.join("state").to_str().context("State path encoding")?,
            if offline { "--offline" } else { "--demo" },
        ]);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env_remove("NO_COLOR");
        for (key, sub) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "xdg-state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
        ] {
            cmd.env(key, root.join(sub));
        }
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || tx.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut result = Self {
            master: pair.master,
            child,
            writer,
            output,
            parser: vt100::Parser::new(58, 212, 0),
            before_local_flags,
        };
        result.read_for(Duration::from_millis(900));
        ensure!(
            result.child.try_wait()?.is_none(),
            "Terminal startup failed: {}",
            result.text()
        );
        Ok(result)
    }
    pub fn read_for(&mut self, duration: Duration) {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(10)) {
                self.parser.process(&bytes);
            }
        }
    }
    pub fn send(&mut self, keys: &str) -> Result<()> {
        self.writer.write_all(keys.as_bytes())?;
        self.writer.flush()?;
        self.read_for(Duration::from_millis(350));
        Ok(())
    }
    pub fn wait_for(&mut self, expected: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.text().contains(expected) && Instant::now() < deadline {
            self.read_for(Duration::from_millis(40));
        }
        ensure!(
            self.text().contains(expected),
            "Expected {expected:?} in terminal: {}",
            self.text()
        );
        Ok(())
    }
    pub fn command(&mut self, command: &str, expected: &str) -> Result<()> {
        let entries = ytui::actions::ENTRIES;
        let entry = entries
            .iter()
            .filter(|entry| {
                if entry.fields.is_empty() {
                    command == entry.command
                } else {
                    command == entry.command
                        || command
                            .strip_prefix(entry.command)
                            .is_some_and(|rest| rest.starts_with(' '))
                }
            })
            .max_by_key(|entry| entry.command.len())
            .ok_or_else(|| anyhow::anyhow!("No UI route for {command}"))?;
        self.send(":")?;
        self.send(entry.title)?;
        // Prefer the exact title when its words also match a longer neighboring action.
        let matches = ytui::actions::matching(entry.title);
        let selected = matches
            .iter()
            .position(|index| entries[*index].title == entry.title)
            .unwrap_or(0);
        for _ in 0..selected {
            self.send("\x1b[B")?;
        }
        self.send("\r")?;
        if !entry.fields.is_empty() {
            let value = command
                .strip_prefix(entry.command)
                .unwrap_or_default()
                .trim();
            self.send(&format!("\x15{value}\r"))?;
        }
        self.wait_for(expected)
    }
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.parser.set_size(rows, cols);
        self.read_for(Duration::from_millis(180));
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        self.send("\x1b")?;
        self.send("q")?;
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait()? {
                ensure!(status.success(), "TUI exited unsuccessfully: {status}");
                ensure!(
                    self.before_local_flags
                        == self.master.get_termios().map(|t| t.local_flags.bits()),
                    "Terminal flags were not restored"
                );
                return Ok(());
            }
            ensure!(Instant::now() < end, "TUI did not exit promptly");
            self.read_for(Duration::from_millis(20));
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
