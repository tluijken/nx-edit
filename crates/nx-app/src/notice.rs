//! What the app tells the user about what it did: the latest message beside the connection state,
//! and the recent ones in a list. A message fades after a while; an error stays until dismissed.

use std::time::{Duration, Instant};

const SHOWN_FOR: Duration = Duration::from_secs(6);
const KEPT: usize = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Debug, Default)]
pub struct Notices {
    log: Vec<Notice>,
    dismissed: bool,
}

impl Notices {
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), false);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), true);
    }

    fn push(&mut self, text: String, error: bool) {
        self.log.push(Notice { text, error, at: Instant::now() });
        if self.log.len() > KEPT {
            self.log.remove(0);
        }
        self.dismissed = false;
    }

    /// The message to show now, if any.
    pub fn current(&self, now: Instant) -> Option<&Notice> {
        let last = self.log.last().filter(|_| !self.dismissed)?;
        (last.error || now.duration_since(last.at) < SHOWN_FOR).then_some(last)
    }

    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// The text of the latest message, shown or not.
    #[cfg(test)]
    pub fn last(&self) -> &str {
        self.log.last().map_or("", |n| n.text.as_str())
    }

    /// The recent messages, newest first.
    pub fn recent(&self) -> impl Iterator<Item = &Notice> {
        self.log.iter().rev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_fades_and_an_error_stays_until_dismissed() {
        let mut notices = Notices::default();
        notices.info("Saved to 03A");
        let later = Instant::now() + SHOWN_FOR + Duration::from_millis(1);
        assert_eq!(notices.current(Instant::now()).map(|n| n.text.as_str()), Some("Saved to 03A"));
        assert!(notices.current(later).is_none());
        notices.error("Cannot move the block");
        assert!(notices.current(later).is_some());
        notices.dismiss();
        assert!(notices.current(Instant::now()).is_none());
        assert_eq!(notices.recent().count(), 2);
    }

    #[test]
    fn only_the_recent_messages_are_kept() {
        let mut notices = Notices::default();
        for i in 0..KEPT + 5 {
            notices.info(format!("{i}"));
        }
        assert_eq!(notices.recent().count(), KEPT);
        assert_eq!(notices.last(), format!("{}", KEPT + 4));
    }
}
