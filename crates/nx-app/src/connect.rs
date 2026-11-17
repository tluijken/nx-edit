//! Finding the HX Stomp: tried on a thread of its own, so the window never waits for USB, and tried
//! again every few seconds until it answers, so plugging it in later or after a loss reconnects.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use nx_device::{device, live};
use nx_hlx::Preset;

use crate::view::Connection;

const RETRY: Duration = Duration::from_secs(3);

/// A session with the pedal and the preset it held on connecting.
pub struct Linked {
    pub connection: Connection,
    pub preset: Preset,
    /// Edits the pedal had not saved when nx-edit connected.
    pub modified: bool,
    /// What went wrong without stopping the connection, such as an unreadable preset list.
    pub notes: Vec<String>,
}

pub type Connect = Arc<dyn Fn() -> Result<Linked, String> + Send + Sync>;

pub struct Connector {
    connect: Connect,
    attempt: Option<Receiver<Result<Linked, String>>>,
    retry_at: Instant,
    /// Why the last attempt failed, in words for the user.
    pub failure: Option<String>,
}

impl Connector {
    pub fn new(connect: Connect) -> Self {
        Self { connect, attempt: None, retry_at: Instant::now(), failure: None }
    }

    /// Starts an attempt when one is due; the pedal, once an attempt found it.
    pub fn poll(&mut self, now: Instant) -> Option<Linked> {
        if let Some(attempt) = &self.attempt {
            let result = match attempt.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err("connecting stopped unexpectedly".to_owned()),
            };
            self.attempt = None;
            return match result {
                Ok(linked) => {
                    self.failure = None;
                    Some(linked)
                }
                Err(e) => {
                    self.failure = Some(explain(&e));
                    self.retry_at = now + RETRY;
                    None
                }
            };
        }
        if now >= self.retry_at {
            let (done, attempt) = mpsc::channel();
            let connect = self.connect.clone();
            let spawned = std::thread::Builder::new().name("nx-connect".into()).spawn(move || _ = done.send(connect()));
            match spawned {
                Ok(_) => self.attempt = Some(attempt),
                Err(e) => {
                    self.failure = Some(format!("cannot start connecting: {e}"));
                    self.retry_at = now + RETRY;
                }
            }
        }
        None
    }

    /// Tries again at the next poll instead of waiting.
    pub fn retry_now(&mut self) {
        self.retry_at = Instant::now();
    }

    pub fn trying(&self) -> bool {
        self.attempt.is_some()
    }
}

/// Connects over USB, reads the preset list and the preset the pedal holds, and starts the worker.
pub fn usb() -> Connect {
    Arc::new(|| {
        let res_dir = nx_catalog::default_res_dir();
        let syms = nx_catalog::ModelSymbols::load(&res_dir.join("Helix.sym")).map_err(|_| "HX Edit's model data is not imported yet".to_owned())?;
        let device::Connected { session, presets, preset } = device::connect_device()?;
        let hlx = preset.to_hlx(&syms)?;
        let mut notes = Vec::new();
        let presets = presets.unwrap_or_else(|e| {
            notes.push(format!("No preset list: {e}"));
            Vec::new()
        });
        let connection = Connection {
            presets,
            addresses: preset.addresses(&syms)?,
            live: live::Live::start(session),
            current: preset.index,
            generation: 0,
            syms,
            document: preset.document.clone(),
        };
        Ok(Linked { connection, preset: hlx, modified: preset.modified, notes })
    })
}

/// A connection error with what the user can do about it.
pub fn explain(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("no hx stomp connected") {
        "No HX Stomp found. Is it plugged in and switched on, and not passed to a virtual machine?".to_owned()
    } else if lower.contains("busy") {
        "The HX Stomp is in use by another program. Close HX Edit and nx-edit tries again.".to_owned()
    } else if lower.contains("permission") || lower.contains("access denied") {
        if cfg!(target_os = "linux") {
            "No permission to open the HX Stomp. Install the udev rule from the README, then replug it.".to_owned()
        } else {
            "No permission to open the HX Stomp.".to_owned()
        }
    } else {
        error.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(connector: &mut Connector) -> Option<Linked> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while connector.attempt.is_some() || connector.failure.is_none() {
            assert!(Instant::now() < deadline, "the attempt never ended");
            if let Some(linked) = connector.poll(Instant::now()) {
                return Some(linked);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn a_failed_attempt_says_why_and_is_tried_again_later() {
        let mut connector = Connector::new(Arc::new(|| Err("HX Stomp: no HX Stomp connected".to_owned())));
        assert!(wait(&mut connector).is_none());
        assert!(connector.failure.as_deref().unwrap().starts_with("No HX Stomp found"));
        assert!(connector.poll(Instant::now()).is_none());
        assert!(!connector.trying(), "not again straight away");
        connector.poll(Instant::now() + RETRY);
        assert!(connector.trying());
    }

    #[test]
    fn retry_now_skips_the_wait() {
        let mut connector = Connector::new(Arc::new(|| Err("busy".to_owned())));
        wait(&mut connector);
        connector.retry_now();
        connector.poll(Instant::now());
        assert!(connector.trying());
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("HX Stomp: Device or resource busy").contains("Close HX Edit"));
        assert_eq!(explain("preset info: timed out"), "preset info: timed out");
    }
}
