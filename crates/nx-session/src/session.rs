use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nx_proto::Request;
use rmpv::Value;

use crate::engine::{Command, Engine};
use crate::{Channel, Config, Error, Event, Transport};

/// An open session with the device. Requests may be issued from any thread; each channel carries
/// one request at a time and the rest queue in order.
pub struct Session {
    commands: Option<Sender<Command>>,
    thread: Option<JoinHandle<()>>,
    device_info: Vec<u8>,
    close_limit: Duration,
}

/// A request that was sent; poll it with [`Pending::try_take`] or block on [`Pending::wait`].
#[must_use]
pub struct Pending(Receiver<Result<Value, Error>>);

impl Pending {
    pub fn wait(self) -> Result<Value, Error> {
        self.0.recv().unwrap_or(Err(Error::SessionClosed))
    }

    pub fn try_take(&self) -> Option<Result<Value, Error>> {
        match self.0.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(Error::SessionClosed)),
        }
    }
}

/// Longest a close can take on the session thread: a request's reply, then its deferred
/// completion, then two channel closes, with room for the poll loop.
fn close_limit(config: &Config) -> Duration {
    config.reply_timeout + config.deferred_timeout + config.handshake_timeout * 2 + config.ack_delay + Duration::from_secs(1)
}

impl Session {
    pub fn connect<T: Transport>(transport: T) -> Result<Self, Error> {
        Self::connect_with(transport, Config::default())
    }

    /// Runs the handshake on a new session thread and returns once every channel is open. A
    /// failed handshake is reported, never retried: the channels it opened are closed the way HX
    /// Edit quits and the transport is dropped with the thread.
    pub fn connect_with<T: Transport>(transport: T, config: Config) -> Result<Self, Error> {
        let close_limit = close_limit(&config);
        let (commands, inbox) = mpsc::channel();
        let (ready_tx, ready) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("nx-session".into())
            .spawn(move || {
                let mut engine = Engine::new(transport, config);
                match engine.handshake() {
                    Ok(info) => {
                        _ = ready_tx.send(Ok(info));
                        engine.run(inbox);
                    }
                    Err(e) => {
                        engine.abandon();
                        _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(Error::from)?;
        let device_info = ready.recv().unwrap_or(Err(Error::SessionClosed))?;
        Ok(Self { commands: Some(commands), thread: Some(thread), device_info, close_limit })
    }

    /// The raw reply to the service 5 probe on `0x1001`; it contains the device's name.
    pub fn device_info(&self) -> &[u8] {
        &self.device_info
    }

    pub fn send(&self, channel: Channel, request: Request) -> Pending {
        let (reply, rx) = mpsc::channel();
        if let Some(c) = &self.commands {
            _ = c.send(Command::Request { channel, request, reply });
        }
        Pending(rx)
    }

    /// A request on the RPC channel; a deferred operation returns once it has completed.
    pub fn request(&self, request: Request) -> Result<Value, Error> {
        self.request_on(Channel::Rpc, request)
    }

    pub fn request_on(&self, channel: Channel, request: Request) -> Result<Value, Error> {
        self.send(channel, request).wait()
    }

    /// A new subscription to the device's notifications from now on.
    pub fn events(&self) -> Receiver<Event> {
        let (tx, rx) = mpsc::channel();
        let (done, registered) = mpsc::channel();
        if self.commands.as_ref().is_some_and(|c| c.send(Command::Subscribe(tx, done)).is_ok()) {
            _ = registered.recv();
        }
        rx
    }

    /// Ends the session the way HX Edit quits: waits for a running request or deferred operation
    /// (up to its timeout), acknowledges what arrived, then closes control and events. Dropping a
    /// session does the same and ignores the outcome.
    pub fn close(mut self) -> Result<(), Error> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), Error> {
        let Some(commands) = self.commands.take() else { return Err(Error::SessionClosed) };
        let (done, rx) = mpsc::channel();
        let mut stuck = false;
        let outcome = if commands.send(Command::Close(done)).is_ok() {
            rx.recv_timeout(self.close_limit).unwrap_or_else(|e| match e {
                mpsc::RecvTimeoutError::Timeout => {
                    stuck = true;
                    Err(Error::Timeout("the session thread to close"))
                }
                mpsc::RecvTimeoutError::Disconnected => Err(Error::SessionClosed),
            })
        } else {
            Err(Error::SessionClosed)
        };
        // A session thread stuck past the limit (a transport that blocks) is left to finish on its
        // own rather than hanging the caller.
        if !stuck && let Some(t) = self.thread.take() {
            _ = t.join();
        }
        outcome
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.commands.is_some() {
            _ = self.finish();
        }
    }
}
