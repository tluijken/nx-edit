//! The editor's open session with the pedal, on a worker thread so the UI never waits on USB. It
//! sends edits (a burst to one parameter collapses to its latest value), preset and snapshot
//! selections and block switches, reads the preset again when the pedal reports a switch or a
//! block change, by nx-edit or by a footswitch, and passes on parameter changes it reports.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nx_proto::Request;
use nx_session::{Error, Event, Session};
use rmpv::Value;

use crate::device::{DevicePreset, preset_from_session};

/// Notifications after which the pedal's preset differs from what was read, so it is read again:
/// preset switched (4) [seen, 02], snapshot loaded (46) [seen, 05], a block changed: enabled,
/// model (49) [seen, 04, live-05].
const READ_AGAIN: [u64; 3] = [4, 46, 49];
/// A parameter changed, by nx-edit or on the pedal: `{106: {98, 29, 26, 28, 119}}` like op 30 [seen,
/// live-05].
const PARAM_CHANGED: u64 = 30;
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
    Snapshot(u64),
    Enabled { slot: u64, enabled: bool },
}

pub struct Live {
    commands: Option<Sender<Command>>,
    errors: Receiver<String>,
    loaded: Receiver<DevicePreset>,
    changes: Receiver<ParamChange>,
    worker: Option<JoinHandle<()>>,
}

impl Live {
    pub fn start(session: Session) -> Self {
        let (commands, inbox) = mpsc::channel();
        let (report, errors) = mpsc::channel();
        let (load, loaded) = mpsc::channel();
        let (change, changes) = mpsc::channel();
        let events = session.events();
        let worker = thread::Builder::new()
            .name("nx-live".into())
            .spawn(move || work(session, inbox, events, Out { report, load, change }))
            .expect("spawning a thread");
        Self { commands: Some(commands), errors, loaded, changes, worker: Some(worker) }
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

    pub fn select_snapshot(&self, index: u64) {
        self.send(Command::Snapshot(index));
    }

    /// Turns a block on or off (op 41); `slot` as op 30 addresses it.
    pub fn set_enabled(&self, slot: u64, enabled: bool) {
        self.send(Command::Enabled { slot, enabled });
    }

    /// Parameter changes the pedal reported since the last call, in order.
    pub fn changes(&self) -> Vec<ParamChange> {
        self.changes.try_iter().collect()
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

/// A parameter value the pedal reported, addressed as op 30 addresses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamChange {
    pub slot: u64,
    pub sub: u64,
    pub param: u64,
    pub value: f64,
}

impl ParamChange {
    fn from_notification(data: &Value) -> Option<Self> {
        let inner = field(data, 106)?;
        let u = |k| field(inner, k).and_then(Value::as_u64);
        Some(Self { slot: u(98)?, sub: u(26)?, param: u(28)?, value: field(inner, 119)?.as_f64()? })
    }
}

fn field(map: &Value, key: u64) -> Option<&Value> {
    map.as_map()?.iter().find(|(k, _)| k.as_u64() == Some(key)).map(|(_, v)| v)
}

/// Where the worker's results go.
struct Out {
    report: Sender<String>,
    load: Sender<DevicePreset>,
    change: Sender<ParamChange>,
}

struct Worker {
    session: Session,
    /// The block op 78 last selected; reading the preset again forgets it.
    selected: Option<(u64, u64)>,
    out: Out,
}

fn work(session: Session, inbox: Receiver<Command>, events: Receiver<Event>, out: Out) {
    let mut worker = Worker { session, selected: None, out };
    loop {
        let mut read_again = false;
        for event in events.try_iter() {
            let Event::Notification { id, data } = event else { continue };
            read_again |= READ_AGAIN.contains(&id);
            if id == PARAM_CHANGED && let Some(change) = ParamChange::from_notification(&data) {
                _ = worker.out.change.send(change);
            }
        }
        if read_again && worker.reload().is_err() {
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
        _ = worker.out.report.send(e.to_string());
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
            _ => None,
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
            Command::Snapshot(index) => self.session.request(Request::SelectSnapshot { index }).map(drop),
            Command::Enabled { slot, enabled } => self.session.request(Request::SetBlockEnabled { block: slot, enabled }).map(drop),
        };
        outcome.map_err(|e| {
            _ = self.out.report.send(format!("{e}; writing stopped"));
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
                _ = self.out.load.send(preset);
                Ok(())
            }
            Err(e) => {
                _ = self.out.report.send(format!("{e}; stopped"));
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

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn switching_snapshot_sends_op_88() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.select_snapshot(2);
        wait_until("op 88 not sent", || sent(&device).contains(&Request::SelectSnapshot { index: 2 }));
    }

    #[test]
    fn switching_a_block_sends_op_41() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set_enabled(5, true);
        wait_until("op 41 not sent", || sent(&device).contains(&Request::SetBlockEnabled { block: 5, enabled: true }));
    }

    #[test]
    fn a_knob_turned_on_the_pedal_is_reported_without_reading_the_preset() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        let change = Value::Map(vec![
            (98.into(), 3.into()),
            (29.into(), true.into()),
            (26.into(), 0.into()),
            (28.into(), 1.into()),
            (119.into(), Value::F32(0.25)),
        ]);
        device.notify(30, Value::Map(vec![(82.into(), 0.into()), (106.into(), change)]));
        let mut changes = Vec::new();
        wait_until("no change reported", || {
            changes.extend(live.changes());
            !changes.is_empty()
        });
        assert_eq!(changes, [ParamChange { slot: 3, sub: 0, param: 1, value: 0.25 }]);
        assert!(sent(&device).is_empty(), "a reported change is not read back");
    }

    #[test]
    fn a_snapshot_or_block_change_on_the_pedal_reads_the_preset_again() {
        let device = pedal(&["Recto DM", "5150 DM", "Revv DM", "Mark Lead"]);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        device.notify(46, Value::Map(vec![(92.into(), 2.into())]));
        wait_for_load(&live);
        let block = Value::Map(vec![(98.into(), 5.into()), (59.into(), true.into())]);
        device.notify(49, Value::Map(vec![(121.into(), 17.into()), (106.into(), block)]));
        wait_for_load(&live);
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
