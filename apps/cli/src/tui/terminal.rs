use super::render::{Line, Tone};
use anyhow::{Result, ensure};
use std::io::Write;

#[cfg(unix)]
#[path = "terminal/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "terminal/windows.rs"]
mod platform;

pub(super) struct Terminal {
    mode: platform::Mode,
    previous: Vec<Line>,
    color: bool,
}

impl Terminal {
    pub fn enter(color: bool) -> Result<Self> {
        ensure!(
            std::env::var("TERM").as_deref() != Ok("dumb"),
            "当前终端不支持 TUI；请使用 cor status"
        );
        let terminal = Self {
            mode: platform::Mode::enter()?,
            previous: Vec::new(),
            color: color && std::env::var_os("NO_COLOR").is_none(),
        };
        let mut output = std::io::stdout().lock();
        output.write_all(b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[2J")?;
        output.flush()?;
        Ok(terminal)
    }

    pub fn size(&self) -> Result<(usize, usize)> {
        self.mode.size()
    }

    pub fn read(&mut self) -> Result<Vec<u8>> {
        self.mode.read()
    }

    pub fn draw(&mut self, frame: &[Line]) -> Result<()> {
        let mut output = std::io::stdout().lock();
        if self.previous.len() != frame.len()
            || self.previous.first().map(|line| line.text.len())
                != frame.first().map(|line| line.text.len())
        {
            output.write_all(b"\x1b[2J")?;
            self.previous.clear();
        }
        for (index, line) in frame.iter().enumerate() {
            if self.previous.get(index) == Some(line) {
                continue;
            }
            let style = if self.color {
                match line.tone {
                    Tone::Plain => "\x1b[0m",
                    Tone::Muted => "\x1b[0;90m",
                    Tone::Accent => "\x1b[0;38;5;79m",
                    Tone::Selected => "\x1b[0;30;48;5;79m",
                    Tone::Warning => "\x1b[0;38;5;215m",
                }
            } else {
                "\x1b[0m"
            };
            write!(
                output,
                "\x1b[{};1H{style}{}\x1b[0m\x1b[K",
                index + 1,
                line.text
            )?;
        }
        output.flush()?;
        self.previous = frame
            .iter()
            .map(|line| Line {
                text: line.text.clone(),
                tone: line.tone,
            })
            .collect();
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut output = std::io::stdout().lock();
        let _ = output.write_all(b"\x1b[0m\x1b[?2004l\x1b[?25h\x1b[?1049l");
        let _ = output.flush();
    }
}
