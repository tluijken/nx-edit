//! The editor's open session with the pedal. Edits go to a worker thread so the UI never waits on
//! USB; a burst of edits to one parameter collapses to its latest value.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use nx_proto::Request;
use nx_session::{Error, Session};
use rmpv::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Edit {
    slot: u64,
    sub: u64,
    param: u64,
    value: f32,
}

pub struct Live {
    edits: Option<Sender<Edit>>,
    errors: Receiver<String>,
    worker: Option<JoinHandle<()>>,
}

impl Live {
    pub fn start(session: Session) -> Self {
        let (edits, inbox) = mpsc::channel();
        let (report, errors) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("nx-live".into())
            .spawn(move || work(session, inbox, report))
            .expect("spawning a thread");
        Self { edits: Some(edits), errors, worker: Some(worker) }
    }

    /// Sets a float parameter, addressed as op 30 addresses it.
    pub fn set(&self, slot: u64, sub: u64, param: u64, value: f32) {
        if let Some(edits) = &self.edits {
            _ = edits.send(Edit { slot, sub, param, value });
        }
    }

    /// Errors reported since the last call.
    pub fn errors(&self) -> Vec<String> {
        self.errors.try_iter().collect()
    }

    fn stop(&mut self) {
        self.edits.take();
        if let Some(worker) = self.worker.take() {
            _ = worker.join();
        }
    }
}

/// Dropping sends what is queued, then closes the session.
impl Drop for Live {
    fn drop(&mut self) {
        self.stop();
    }
}

fn work(session: Session, inbox: Receiver<Edit>, report: Sender<String>) {
    let mut selected = None;
    while let Ok(first) = inbox.recv() {
        let mut batch = vec![first];
        for edit in inbox.try_iter() {
            match batch.iter_mut().find(|e| (e.slot, e.sub, e.param) == (edit.slot, edit.sub, edit.param)) {
                Some(queued) => queued.value = edit.value,
                None => batch.push(edit),
            }
        }
        for edit in batch {
            // A refusal means nx-edit addressed something it does not understand yet; sending
            // more of the same is how a dragged knob turned into two dozen refused edits.
            if let Err(e) = apply(&session, &mut selected, edit) {
                _ = report.send(format!("{e}; writing stopped"));
                return;
            }
        }
    }
    if let Err(e) = session.close() {
        _ = report.send(e.to_string());
    }
}

fn apply(session: &Session, selected: &mut Option<(u64, u64)>, edit: Edit) -> Result<(), Error> {
    if *selected != Some((edit.slot, edit.sub)) {
        session.request(Request::SelectBlock { block: edit.slot, path: edit.sub })?;
        *selected = Some((edit.slot, edit.sub));
    }
    session.request(Request::SetParam { block: edit.slot, path: edit.sub, param: edit.param, value: Value::F32(edit.value) })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use nx_proto::Request;
    use nx_session::fake::{FakeDevice, Reply};
    use nx_session::{Channel, Session};
    use rmpv::Value;

    use super::*;

    fn device(reply: impl Fn(&Request) -> Reply + Send + 'static) -> FakeDevice {
        let device = FakeDevice::new();
        device.strict(true);
        device.on_request(move |_, _, req| reply(req));
        device
    }

    fn echo(req: &Request) -> Reply {
        match req {
            Request::SetParam { block, path, param, value } => Reply::Ok(Value::Map(vec![
                (98.into(), (*block).into()),
                (26.into(), (*path).into()),
                (28.into(), (*param).into()),
                (119.into(), value.clone()),
            ])),
            _ => Reply::Ok(Value::Nil),
        }
    }

    fn sent(device: &FakeDevice) -> Vec<Request> {
        device.requests().into_iter().filter(|(c, _, _)| *c == Channel::Rpc).map(|(_, _, r)| r).collect()
    }

    #[test]
    fn an_edit_selects_the_block_then_sets_the_parameter() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(3, 0, 0, 0.5);
        live.set(3, 0, 0, 0.55);
        drop(live);
        let sent = sent(&device);
        assert_eq!(sent[0], Request::SelectBlock { block: 3, path: 0 });
        assert_eq!(sent.iter().filter(|r| matches!(r, Request::SelectBlock { .. })).count(), 1);
        assert_eq!(sent.last(), Some(&Request::SetParam { block: 3, path: 0, param: 0, value: Value::F32(0.55) }));
    }

    #[test]
    fn an_edit_on_another_block_selects_that_block_first() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(3, 0, 0, 0.5);
        live.set(6, 1, 2, 0.25);
        drop(live);
        let sent = sent(&device);
        let at = sent.iter().position(|r| *r == Request::SelectBlock { block: 6, path: 1 }).unwrap();
        assert_eq!(sent[at + 1], Request::SetParam { block: 6, path: 1, param: 2, value: Value::F32(0.25) });
    }

    #[test]
    fn the_last_value_of_a_burst_is_the_one_that_stays() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        for i in 0..=50 {
            live.set(3, 0, 0, i as f32 / 100.0);
        }
        drop(live);
        let last = sent(&device).into_iter().rev().find(|r| matches!(r, Request::SetParam { .. })).unwrap();
        assert_eq!(last, Request::SetParam { block: 3, path: 0, param: 0, value: Value::F32(0.5) });
    }

    #[test]
    fn nothing_more_is_sent_after_a_refusal() {
        let device = device(|req| match req {
            Request::SetParam { .. } => Reply::Status(255, Value::Nil),
            _ => Reply::Ok(Value::Nil),
        });
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(17, 0, 3, 0.45);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while live.errors().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        live.set(17, 0, 3, 1.8);
        live.set(3, 0, 0, 0.5);
        drop(live);
        let sets = sent(&device).into_iter().filter(|r| matches!(r, Request::SetParam { .. })).count();
        assert_eq!(sets, 1);
    }

    #[test]
    fn a_refused_edit_is_reported() {
        let device = device(|req| match req {
            Request::SetParam { .. } => Reply::Status(5, Value::Nil),
            _ => Reply::Ok(Value::Nil),
        });
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(3, 0, 0, 0.5);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut errors = Vec::new();
        while errors.is_empty() && std::time::Instant::now() < deadline {
            errors = live.errors();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(errors.iter().any(|e| e.contains("status 5")), "{errors:?}");
        drop(live);
    }
}
