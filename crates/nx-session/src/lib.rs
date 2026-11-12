//! The session layer on the HX Stomp's editor interface: channel handshake, sequence and
//! acknowledgement bookkeeping, keep-alives, and RPC over the opened channels. See the "Session"
//! section of docs/protocol.md for what this is built on and how sure we are of each rule.
//!
//! A [`Session`] owns its [`Transport`] on a thread of its own and talks to callers over channels,
//! so it needs no async runtime and can be hosted by any.

mod engine;
mod link;
mod session;

#[cfg(feature = "fake")]
pub mod fake;

use std::io;
use std::sync::Arc;
use std::time::Duration;

pub use session::{Pending, Session};

/// Whole bulk transfers on the editor interface (bulk OUT `0x01`, IN `0x81`).
///
/// `read` must keep a read posted between calls: the device stops draining its IN queue when
/// nobody reads, and then stops accepting writes as well [map].
pub trait Transport: Send + 'static {
    fn write(&mut self, transfer: &[u8]) -> io::Result<()>;
    /// One transfer, or `None` if none arrived within `timeout`.
    fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>>;
}

/// The three channels HX Edit opens, named by role; values are the host endpoint ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// `0x1001`: device info (service 5), then presets and setlists (service 2).
    Control,
    /// `0x1080`, service 6: editing RPC.
    Rpc,
    /// `0x1002`, service 4: notifications from the device.
    Events,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Control, Channel::Rpc, Channel::Events];

    pub fn host_id(self) -> u16 {
        match self {
            Self::Control => 0x1001,
            Self::Rpc => 0x1080,
            Self::Events => 0x1002,
        }
    }

    pub fn device_id(self) -> u16 {
        match self {
            Self::Control => 0x03ef,
            Self::Rpc => 0x03ed,
            Self::Events => 0x03f0,
        }
    }

    pub fn from_host_id(id: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.host_id() == id)
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// How long a request may wait for its reply.
    pub reply_timeout: Duration,
    /// How long a deferred operation (status 1) may wait for notification 20.
    pub deferred_timeout: Duration,
    /// How long each handshake step may wait for the device.
    pub handshake_timeout: Duration,
    /// How long a received message waits for a request to carry its acknowledgement before a bare
    /// ACK goes out. Partial messages are acknowledged at once.
    pub ack_delay: Duration,
    /// A channel the host has sent nothing on for this long gets a keep-alive.
    pub keepalive_interval: Duration,
    /// How long the service 5 probe of `0x1001` stays open before it is closed.
    pub probe_linger: Duration,
    /// Read timeout of the session loop; bounds how late commands and timers are handled.
    pub poll: Duration,
    /// Upper bound on draining stale transfers before the handshake.
    pub drain_limit: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reply_timeout: Duration::from_secs(2),
            deferred_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(1),
            ack_delay: Duration::from_millis(10),
            keepalive_interval: Duration::from_secs(1),
            probe_linger: Duration::from_secs(1),
            poll: Duration::from_millis(5),
            drain_limit: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `{105: id, 106: data}` on the events channel.
    Notification { id: u64, data: rmpv::Value },
    /// The device ended a channel; requests on it fail from now on.
    ChannelClosed(Channel),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("transport: {0}")]
    Transport(Arc<io::Error>),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("handshake on {channel:?} failed: {reason}")]
    Handshake { channel: Channel, reason: String },
    /// A HELLO was asked for on a channel that is open, or a second handshake on one session.
    #[error("{0:?} is already open; a session handshakes once")]
    AlreadyOpen(Channel),
    #[error("device kept sending for longer than the drain limit before the handshake")]
    Backlog,
    #[error("request failed with status {status}: {result}")]
    Status { status: u64, result: rmpv::Value },
    #[error("the device closed {0:?}")]
    ChannelClosed(Channel),
    #[error("session closed")]
    SessionClosed,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Transport(Arc::new(e))
    }
}
