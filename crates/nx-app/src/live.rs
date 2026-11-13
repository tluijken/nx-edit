//! The editor's open session with the pedal, on a worker thread so the UI never waits on USB. It
//! sends edits (a burst to one parameter collapses to its latest value) and preset selections, and
//! reads a preset again whenever the pedal reports it switched, by nx-edit or by a footswitch.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nx_proto::Request;
use nx_session::{Error, Event, Session};
use rmpv::Value;

use crate::device::{DevicePreset, preset_from_session};

/// Notification the pedal sends once it has switched preset, `{106: {107: setlist, 108: index}}`
/// [seen, 02].
const PRESET_SWITCHED: u64 = 4;
/// How often the worker looks at notifications while no command comes in.
const POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Edit {
    slot: u64,
    sub: u64,
    param: u64,
    value: f32,
}

enum Command {
    Edit(Edit),
    Select { setlist: u64, index: u64 },
}

pub struct Live {
    commands: Option<Sender<Command>>,
    errors: Receiver<String>,
    loaded: Receiver<DevicePreset>,
    worker: Option<JoinHandle<()>>,
}

impl Live {
    pub fn start(session: Session) -> Self {
        let (commands, inbox) = mpsc::channel();
        let (report, errors) = mpsc::channel();
        let (load, loaded) = mpsc::channel();
        let events = session.events();
        let worker = thread::Builder::new()
            .name("nx-live".into())
            .spawn(move || work(session, inbox, events, report, load))
            .expect("spawning a thread");
        Self { commands: Some(commands), errors, loaded, worker: Some(worker) }
    }

    /// Sets a float parameter, addressed as op 30 addresses it.
    pub fn set(&self, slot: u64, sub: u64, param: u64, value: f32) {
        self.send(Command::Edit(Edit { slot, sub, param, value }));
    }

    /// Switches the pedal to a preset; it arrives through [`Live::loaded`] once the pedal has
    /// switched. Unsaved edits on the pedal are lost, as with a footswitch.
    pub fn select(&self, setlist: u64, index: u64) {
        self.send(Command::Select { setlist, index });
    }

    /// The latest preset read after a switch, if one arrived since the last call.
    pub fn loaded(&self) -> Option<DevicePreset> {
        self.loaded.try_iter().last()
    }

    /// Errors reported since the last call.
    pub fn errors(&self) -> Vec<String> {
        self.errors.try_iter().collect()
    }

    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            _ = commands.send(command);
        }
    }

    fn stop(&mut self) {
        self.commands.take();
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

struct Worker {
    session: Session,
    /// The block op 78 last selected; a preset switch forgets it.
    selected: Option<(u64, u64)>,
    report: Sender<String>,
    load: Sender<DevicePreset>,
}

fn work(session: Session, inbox: Receiver<Command>, events: Receiver<Event>, report: Sender<String>, load: Sender<DevicePreset>) {
    let mut worker = Worker { session, selected: None, report, load };
    loop {
        let switched = events.try_iter().any(|e| matches!(e, Event::Notification { id: PRESET_SWITCHED, .. }));
        if switched && worker.reload().is_err() {
            return;
        }
        let first = match inbox.recv_timeout(POLL) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for command in coalesce(first, &inbox) {
            if worker.run(command).is_err() {
                return;
            }
        }
    }
    if let Err(e) = worker.session.close() {
        _ = worker.report.send(e.to_string());
    }
}

/// The first command and whatever queued behind it, with each parameter's edits reduced to the last.
fn coalesce(first: Command, inbox: &Receiver<Command>) -> Vec<Command> {
    let mut batch = vec![first];
    for command in inbox.try_iter() {
        let queued = match &command {
            Command::Edit(edit) => batch.iter_mut().rev().take_while(|c| matches!(c, Command::Edit(_))).find_map(|c| match c {
                Command::Edit(e) if (e.slot, e.sub, e.param) == (edit.slot, edit.sub, edit.param) => Some(e),
                _ => None,
            }),
            Command::Select { .. } => None,
        };
        match (queued, command) {
            (Some(queued), Command::Edit(edit)) => queued.value = edit.value,
            (_, command) => batch.push(command),
        }
    }
    batch
}

impl Worker {
    /// Any failure is reported and ends the worker: a refusal means nx-edit addressed something it
    /// does not understand yet, and sending more of the same is how a dragged knob turned into two
    /// dozen refused edits.
    fn run(&mut self, command: Command) -> Result<(), ()> {
        let outcome = match command {
            Command::Edit(edit) => self.apply(edit),
            Command::Select { setlist, index } => self.session.request(Request::SelectPreset { setlist, index }).map(drop),
        };
        outcome.map_err(|e| {
            _ = self.report.send(format!("{e}; writing stopped"));
        })
    }

    fn apply(&mut self, edit: Edit) -> Result<(), Error> {
        if self.selected != Some((edit.slot, edit.sub)) {
            self.session.request(Request::SelectBlock { block: edit.slot, path: edit.sub })?;
            self.selected = Some((edit.slot, edit.sub));
        }
        self.session.request(Request::SetParam { block: edit.slot, path: edit.sub, param: edit.param, value: Value::F32(edit.value) })?;
        Ok(())
    }

    fn reload(&mut self) -> Result<(), ()> {
        self.selected = None;
        match preset_from_session(&self.session) {
            Ok(preset) => {
                _ = self.load.send(preset);
                Ok(())
            }
            Err(e) => {
                _ = self.report.send(format!("{e}; stopped"));
                Err(())
            }
        }
    }
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

    fn pedal(names: &'static [&'static str]) -> FakeDevice {
        device(move |req| match req {
            Request::PresetInfo => Reply::Ok(Value::Map(vec![(108.into(), 3.into()), (109.into(), format!("{}\0", names[3]).into())])),
            Request::PresetDocument => Reply::Ok(Value::from("document")),
            _ => Reply::Ok(Value::Nil),
        })
    }

    fn switched_to(device: &FakeDevice, index: u64) {
        let place = Value::Map(vec![(107.into(), 0.into()), (108.into(), index.into())]);
        device.notify(4, Value::Map(vec![(82.into(), 1.into()), (106.into(), place)]));
    }

    fn wait_for_load(live: &Live) -> crate::device::DevicePreset {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(preset) = live.loaded() {
                return preset;
            }
            assert!(std::time::Instant::now() < deadline, "no preset loaded; errors {:?}", live.errors());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn selecting_a_preset_sends_op_20_and_loads_it_when_the_pedal_has_switched() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.select(0, 3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !sent(&device).contains(&Request::SelectPreset { setlist: 0, index: 3 }) {
            assert!(std::time::Instant::now() < deadline, "op 20 not sent");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        switched_to(&device, 3);
        let preset = wait_for_load(&live);
        assert_eq!((preset.name.as_str(), preset.index), ("Mark Lead", Some(3)));
        assert_eq!(preset.document, b"document");
    }

    #[test]
    fn a_preset_switched_on_the_pedal_is_loaded() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        switched_to(&device, 3);
        assert_eq!(wait_for_load(&live).name, "Mark Lead");
        let asked: Vec<u64> = sent(&device).iter().map(Request::opcode).collect();
        assert_eq!(asked, vec![23, 22], "a switch on the pedal is only read, never answered with a write");
    }

    #[test]
    fn after_a_switch_an_edit_selects_its_block_again() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(3, 0, 0, 0.5);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !sent(&device).iter().any(|r| matches!(r, Request::SetParam { .. })) {
            assert!(std::time::Instant::now() < deadline, "first edit not sent");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        switched_to(&device, 3);
        wait_for_load(&live);
        live.set(3, 0, 0, 0.6);
        drop(live);
        let selects = sent(&device).iter().filter(|r| matches!(r, Request::SelectBlock { .. })).count();
        assert_eq!(selects, 2);
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
