//! An RAII spinner guard. Construct one before a long-running step; on success
//! call [`Spinner::finish`] to leave a completion line. On any early return
//! (a `?` inside the guard's scope), the `Drop` impl clears the spinner
//! automatically — so error paths can never leak a live spinner.

use std::borrow::Cow;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

pub(crate) struct Spinner {
    pb: ProgressBar,
    finished: bool,
}

impl Spinner {
    pub(crate) fn new(msg: &str) -> Self {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(80));
        Self {
            pb,
            finished: false,
        }
    }

    /// Stop the spinner and leave `msg` in its place (the success path).
    pub(crate) fn finish(mut self, msg: impl Into<Cow<'static, str>>) {
        self.pb.finish_with_message(msg);
        self.finished = true; // suppress the clear in Drop
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if !self.finished {
            self.pb.finish_and_clear();
        }
    }
}
