//! Lines the observer types while a probe command runs, including the
//! request to stop early.

use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;

/// The observer typed `quit` to stop the command early.
#[derive(Debug)]
pub(crate) struct Quit;

impl fmt::Display for Quit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("stopped by the observer")
    }
}

impl std::error::Error for Quit {}

/// Standard input read on a background thread, so long-running steps can
/// check for `quit` without blocking.
///
/// Clones share the same input, so consecutive cases in a suite read one
/// stream.
#[derive(Clone)]
pub(crate) struct Observer {
    lines: Arc<Mutex<Receiver<String>>>,
}

impl Observer {
    pub(crate) fn spawn() -> Self {
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || forward_lines(&sender));
        Self {
            lines: Arc::new(Mutex::new(lines)),
        }
    }

    fn lines(&self) -> anyhow::Result<MutexGuard<'_, Receiver<String>>> {
        self.lines
            .lock()
            .map_err(|_poisoned| anyhow::anyhow!("observer input lock poisoned"))
    }

    /// Wait for the next line.
    ///
    /// # Errors
    ///
    /// Returns [`Quit`] if the line is `quit`, or an error if standard input
    /// closed.
    pub(crate) fn next_line(&self) -> anyhow::Result<String> {
        let line = self
            .lines()?
            .recv()
            .map_err(|_closed| anyhow::anyhow!("standard input closed"))?;
        if is_quit(&line) {
            return Err(Quit.into());
        }
        Ok(line)
    }

    /// Return [`Quit`] if the observer typed `quit` since the last check,
    /// discarding any other lines.
    pub(crate) fn check_quit(&self) -> anyhow::Result<()> {
        let lines = self.lines()?;
        loop {
            match lines.try_recv() {
                Ok(line) if is_quit(&line) => return Err(Quit.into()),
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }
}

fn forward_lines(sender: &Sender<String>) {
    for line in std::io::stdin().lines().map_while(Result::ok) {
        if sender.send(line).is_err() {
            return;
        }
    }
}

fn is_quit(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "q" | "quit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case("quit", true; "word")]
    #[test_case(" QUIT \r", true; "case and whitespace")]
    #[test_case("q", true; "letter")]
    #[test_case("y", false; "answer")]
    #[test_case("", false; "empty line")]
    fn recognizes_quit(line: &str, expected: bool) {
        assert_eq!(is_quit(line), expected);
    }
}
