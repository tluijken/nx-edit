//! The editor's open session with the pedal, on a worker thread so the UI never waits on USB. It
//! sends edits (a burst to one parameter collapses to its latest value), preset and snapshot
//! selections and block switches, reads the preset again when the pedal reports a switch or a
//! block change, by nx-edit or by a footswitch, and passes on parameter changes it reports.
//!
//! Addresses (slot, parameter index) belong to one version of the preset. Every switch or reload
//! starts a new generation; edits carry the generation of the preset they were made on, and the
//! worker drops those made on an older one, so a knob never writes into a preset it was not shown.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nx_proto::Request;
use nx_session::{Error, Event, Session};
use rmpv::Value;

use crate::device::{DevicePreset, Param, preset_from_session};

/// Notifications after which the pedal's preset differs from what was read, so it is read again:
/// preset switched (4) [seen, 02], snapshot loaded (46) [seen, 05], a block changed: enabled,
/// model (49) [seen, 04, live-05], a document written with op 21 (21) [seen, 11].
const READ_AGAIN: [u64; 4] = [4, 46, 49, 21];
/// A preset switch is under way (8), before 4 [seen, 02]; addresses are stale from here on.
const SWITCHING: u64 = 8;
/// A parameter changed, by nx-edit or on the pedal: `{106: {98, 29, 26, 28, 119}}` like op 30 [seen,
/// live-05].
const PARAM_CHANGED: u64 = 30;
/// A block was selected, by op 78 or on the pedal: `{106: {98, 26}}` [seen, 04, live-05].
const BLOCK_SELECTED: u64 = 39;
/// How often the worker looks at notifications while no command comes in.
const POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq)]
struct Edit {
    generation: u64,
    slot: u64,
    sub: u64,
    param: Param,
    value: Value,
}

enum Command {
    Edit(Edit),
    Select { setlist: u64, index: u64 },
    Snapshot(u64),
    SnapshotColor { index: u64, color: u64 },
    Enabled { generation: u64, slot: u64, enabled: bool },
    /// A block inserted, given another model or removed, on the preset of `generation`.
    Chain { generation: u64, request: Request },
    Save { setlist: u64, index: u64, name: String },
    Assign { setlist: u64, index: u64, name: String, document: Vec<u8> },
}

/// A preset read from the pedal, and the generation edits made on it must carry.
#[derive(Debug)]
pub struct Loaded {
    pub generation: u64,
    pub preset: DevicePreset,
}

/// What the pedal reported, in the order it reported it.
#[derive(Debug)]
pub enum Update {
    Loaded(Loaded),
    /// A parameter change, with the generation of the preset it was made on.
    Changed(u64, ParamChange),
    /// The pedal stored its edits at this place (op 71).
    Saved(u64),
    /// A preset was written to this place (op 8), with the preset list read after it.
    Assigned { index: u64, presets: Vec<nx_proto::PresetEntry> },
}

pub struct Live {
    commands: Option<Sender<Command>>,
    errors: Receiver<String>,
    updates: Receiver<Update>,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Live {
    /// Starts the worker on the preset read when connecting, generation 0.
    pub fn start(session: Session) -> Self {
        let (commands, inbox) = mpsc::channel();
        let (report, errors) = mpsc::channel();
        let (update, updates) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let events = session.events();
        let flag = running.clone();
        let worker = thread::Builder::new()
            .name("nx-live".into())
            .spawn(move || {
                work(session, inbox, events, Out { report, update });
                flag.store(false, Ordering::SeqCst);
            })
            .expect("spawning a thread");
        Self { commands: Some(commands), errors, updates, running, worker: Some(worker) }
    }

    /// Sets a float parameter, addressed as op 30 addresses it, on the preset of `generation`.
    pub fn set(&self, generation: u64, slot: u64, sub: u64, param: u64, value: f32) {
        self.set_value(generation, slot, sub, Param::Model(param), Value::F32(value));
    }

    /// Sets any value op 30 can carry, a float, an integer or a switch, on the preset of
    /// `generation`.
    pub fn set_value(&self, generation: u64, slot: u64, sub: u64, param: Param, value: Value) {
        self.send(Command::Edit(Edit { generation, slot, sub, param, value }));
    }

    /// Switches the pedal to a preset; it arrives through [`Live::updates`] once the pedal has
    /// switched. Unsaved edits on the pedal are lost, as with a footswitch.
    pub fn select(&self, setlist: u64, index: u64) {
        self.send(Command::Select { setlist, index });
    }

    pub fn select_snapshot(&self, index: u64) {
        self.send(Command::Snapshot(index));
    }

    /// Sets a snapshot's LED colour (op 98).
    pub fn set_snapshot_color(&self, index: u64, color: u64) {
        self.send(Command::SnapshotColor { index, color });
    }

    /// Turns a block on or off (op 41) on the preset of `generation`; `slot` as op 30 addresses it.
    pub fn set_enabled(&self, generation: u64, slot: u64, enabled: bool) {
        self.send(Command::Enabled { generation, slot, enabled });
    }

    /// Puts `model` into the empty `slot` of the preset of `generation` (op 39). The pedal then
    /// reports a block change, and the preset is read again.
    pub fn insert(&self, generation: u64, slot: u64, model: u32, engine: u64) {
        self.send(Command::Chain { generation, request: Request::InsertBlock { block: slot, model, engine, cab: None } });
    }

    /// Gives the block in `slot` another model (op 40); the pedal picks its engine.
    pub fn change_model(&self, generation: u64, slot: u64, model: u32) {
        self.send(Command::Chain { generation, request: Request::ChangeModel { block: slot, model, cab: None } });
    }

    /// Empties `slot` (op 28).
    pub fn remove(&self, generation: u64, slot: u64) {
        self.send(Command::Chain { generation, request: Request::RemoveBlock { block: slot } });
    }

    /// Replaces the preset of `generation` on the pedal with `document` (op 21), as HX Edit does to
    /// move a block. The pedal confirms with notifications 20 and 21, and the preset is read again.
    pub fn write_document(&self, generation: u64, document: Vec<u8>) {
        self.send(Command::Chain { generation, request: Request::WriteDocument { document } });
    }

    /// Stores the pedal's edits at a place under `name` (op 71), reported as [`Update::Saved`].
    pub fn save(&self, setlist: u64, index: u64, name: &str) {
        self.send(Command::Save { setlist, index, name: name.to_owned() });
    }

    /// Writes a whole preset to a place under `name` (librarian op 8), as HX Edit imports a file;
    /// reported as [`Update::Assigned`] with the preset list read again.
    pub fn assign(&self, setlist: u64, index: u64, name: &str, document: Vec<u8>) {
        self.send(Command::Assign { setlist, index, name: name.to_owned(), document });
    }

    /// What the pedal reported since the last call, in order.
    pub fn updates(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }

    /// Errors reported since the last call.
    pub fn errors(&self) -> Vec<String> {
        self.errors.try_iter().collect()
    }

    /// False once the worker has stopped, after a refusal or a lost connection; nothing is sent
    /// any more.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
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
#[derive(Debug, Clone, PartialEq)]
pub struct ParamChange {
    pub slot: u64,
    pub sub: u64,
    pub param: Param,
    pub value: Value,
}

impl ParamChange {
    fn from_notification(data: &Value) -> Option<Self> {
        let inner = field(data, 106)?;
        let u = |k| field(inner, k).and_then(Value::as_u64);
        let at = u(28)?;
        let param = if field(inner, 29)?.as_bool()? { Param::Model(at) } else { Param::Extra(at) };
        Some(Self { slot: u(98)?, sub: u(26)?, param, value: field(inner, 119)?.clone() })
    }
}

fn field(map: &Value, key: u64) -> Option<&Value> {
    map.as_map()?.iter().find(|(k, _)| k.as_u64() == Some(key)).map(|(_, v)| v)
}

/// Where the worker's results go.
struct Out {
    report: Sender<String>,
    update: Sender<Update>,
}

struct Worker {
    session: Session,
    /// The block selected on the pedal, by op 78 or as the pedal reported; reading the preset
    /// again forgets it.
    selected: Option<(u64, u64)>,
    generation: u64,
    out: Out,
}

fn work(session: Session, inbox: Receiver<Command>, events: Receiver<Event>, out: Out) {
    let mut worker = Worker { session, selected: None, generation: 0, out };
    loop {
        if worker.notifications(&events).is_err() {
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
                Command::Edit(e) if (e.generation, e.slot, e.sub, e.param) == (edit.generation, edit.slot, edit.sub, edit.param) => Some(e),
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
    /// Handles what the pedal reported, in order: parameter changes are passed on with the
    /// generation they belong to, a switch or block change starts a new generation and has the
    /// preset read again once.
    fn notifications(&mut self, events: &Receiver<Event>) -> Result<(), ()> {
        let mut read_again = false;
        for event in events.try_iter() {
            let Event::Notification { id, data } = event else { continue };
            match id {
                PARAM_CHANGED => {
                    if let Some(change) = ParamChange::from_notification(&data) {
                        _ = self.out.update.send(Update::Changed(self.generation, change));
                    }
                }
                BLOCK_SELECTED => {
                    let inner = field(&data, 106);
                    let u = |k| inner.and_then(|i| field(i, k)).and_then(Value::as_u64);
                    self.selected = u(98).zip(u(26));
                }
                SWITCHING => self.next_generation(),
                id if READ_AGAIN.contains(&id) => {
                    self.next_generation();
                    read_again = true;
                }
                _ => {}
            }
        }
        if read_again { self.reload() } else { Ok(()) }
    }

    fn next_generation(&mut self) {
        self.generation += 1;
        self.selected = None;
    }

    /// Any failure is reported and ends the worker: a refusal means nx-edit addressed something it
    /// does not understand yet, and sending more of the same is how a dragged knob turned into two
    /// dozen refused edits. Edits and block switches made on an older preset are dropped.
    fn run(&mut self, command: Command) -> Result<(), ()> {
        let outcome = match command {
            Command::Edit(edit) if edit.generation == self.generation => self.apply(edit),
            Command::Enabled { generation, slot, enabled } if generation == self.generation => {
                self.session.request(Request::SetBlockEnabled { block: slot, enabled }).map(drop)
            }
            Command::Chain { generation, request } if generation == self.generation => self.session.request(request).map(drop),
            Command::Edit(_) | Command::Enabled { .. } | Command::Chain { .. } => Ok(()),
            Command::Select { setlist, index } => {
                let outcome = self.session.request(Request::SelectPreset { setlist, index }).map(drop);
                self.next_generation();
                outcome
            }
            Command::Snapshot(index) => self.session.request(Request::SelectSnapshot { index }).map(drop),
            Command::SnapshotColor { index, color } => self.session.request(Request::SetSnapshotColor { index, color }).map(drop),
            Command::Save { setlist, index, name } => self.session.request(Request::SavePreset { setlist, index, name }).map(|_| {
                _ = self.out.update.send(Update::Saved(index));
            }),
            Command::Assign { setlist, index, name, document } => self.assign(setlist, index, name, document),
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
        let (block, path, value) = (edit.slot, edit.sub, edit.value);
        self.session.request(match edit.param {
            Param::Model(param) => Request::SetParam { block, path, param, value },
            Param::Extra(index) => Request::SetExtra { block, path, index, value },
        })?;
        Ok(())
    }

    /// Op 255, op 8, op 254 and the preset list again, as HX Edit imports a file [seen, 19].
    fn assign(&mut self, setlist: u64, index: u64, name: String, document: Vec<u8>) -> Result<(), Error> {
        use nx_session::Channel::Control;
        self.session.request_on(Control, Request::ControlBegin)?;
        let written = self.session.request_on(Control, Request::WritePreset { setlist, index, name, document });
        self.session.request_on(Control, Request::ControlStart)?;
        written?;
        let list = self.session.request_on(Control, Request::ListPresets { setlist })?;
        let presets = nx_proto::parse_preset_list(&list).unwrap_or_default();
        _ = self.out.update.send(Update::Assigned { index, presets });
        Ok(())
    }

    fn reload(&mut self) -> Result<(), ()> {
        match preset_from_session(&self.session) {
            Ok(preset) => {
                _ = self.out.update.send(Update::Loaded(Loaded { generation: self.generation, preset }));
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

    fn sets(device: &FakeDevice) -> usize {
        sent(device).iter().filter(|r| matches!(r, Request::SetParam { .. })).count()
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn an_edit_selects_the_block_then_sets_the_parameter() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(0, 3, 0, 0, 0.5);
        live.set(0, 3, 0, 0, 0.55);
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
        live.set(0, 3, 0, 0, 0.5);
        live.set(0, 6, 1, 2, 0.25);
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
            live.set(0, 3, 0, 0, i as f32 / 100.0);
        }
        drop(live);
        let last = sent(&device).into_iter().rev().find(|r| matches!(r, Request::SetParam { .. })).unwrap();
        assert_eq!(last, Request::SetParam { block: 3, path: 0, param: 0, value: Value::F32(0.5) });
    }

    fn pedal(names: &'static [&'static str]) -> FakeDevice {
        device(move |req| match req {
            Request::PresetInfo => Reply::Ok(Value::Map(vec![(108.into(), 3.into()), (109.into(), format!("{}\0", names[3]).into())])),
            Request::PresetDocument => Reply::Ok(Value::from("document")),
            _ => echo(req),
        })
    }

    const NAMES: &[&str] = &["Recto DM", "5150 DM", "Revv DM", "Mark Lead"];

    fn switched_to(device: &FakeDevice, index: u64) {
        let place = Value::Map(vec![(107.into(), 0.into()), (108.into(), index.into())]);
        device.notify(4, Value::Map(vec![(82.into(), 1.into()), (106.into(), place)]));
    }

    fn param_changed(device: &FakeDevice, slot: u64, param: u64, value: f32) {
        let change = Value::Map(vec![
            (98.into(), slot.into()),
            (29.into(), true.into()),
            (26.into(), 0.into()),
            (28.into(), param.into()),
            (119.into(), Value::F32(value)),
        ]);
        device.notify(30, Value::Map(vec![(82.into(), 0.into()), (106.into(), change)]));
    }

    /// Updates until one is a loaded preset; all of them, in order.
    fn updates_until_load(live: &Live) -> Vec<Update> {
        let mut seen = Vec::new();
        wait_until("no preset loaded", || {
            seen.extend(live.updates());
            seen.iter().any(|u| matches!(u, Update::Loaded(_)))
        });
        seen
    }

    fn wait_for_load(live: &Live) -> Loaded {
        updates_until_load(live)
            .into_iter()
            .find_map(|u| match u {
                Update::Loaded(loaded) => Some(loaded),
                Update::Changed(..) | Update::Saved(_) | Update::Assigned { .. } => None,
            })
            .unwrap()
    }

    #[test]
    fn selecting_a_preset_sends_op_20_and_loads_it_when_the_pedal_has_switched() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.select(0, 3);
        wait_until("op 20 not sent", || sent(&device).contains(&Request::SelectPreset { setlist: 0, index: 3 }));
        switched_to(&device, 3);
        let loaded = wait_for_load(&live);
        assert_eq!((loaded.preset.name.as_str(), loaded.preset.index), ("Mark Lead", Some(3)));
        assert_eq!(loaded.preset.document, b"document");
    }

    #[test]
    fn a_preset_switched_on_the_pedal_is_loaded() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        switched_to(&device, 3);
        assert_eq!(wait_for_load(&live).preset.name, "Mark Lead");
        let asked: Vec<u64> = sent(&device).iter().map(Request::opcode).collect();
        assert_eq!(asked, vec![23, 22], "a switch on the pedal is only read, never answered with a write");
    }

    #[test]
    fn edits_meant_for_the_preset_before_a_switch_are_dropped() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.select(0, 3);
        live.set(0, 3, 0, 0, 0.5);
        wait_until("op 20 not sent", || sent(&device).contains(&Request::SelectPreset { setlist: 0, index: 3 }));
        switched_to(&device, 3);
        let loaded = wait_for_load(&live);
        live.set(0, 3, 0, 0, 0.55);
        live.set_enabled(0, 5, true);
        live.set(loaded.generation, 3, 0, 0, 0.6);
        drop(live);
        let sent = sent(&device);
        assert_eq!(sets(&device), 1, "only the edit made on the loaded preset is sent: {sent:?}");
        assert!(sent.contains(&Request::SetParam { block: 3, path: 0, param: 0, value: Value::F32(0.6) }));
        assert!(!sent.iter().any(|r| matches!(r, Request::SetBlockEnabled { .. })));
    }

    #[test]
    fn after_a_switch_an_edit_selects_its_block_again() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(0, 3, 0, 0, 0.5);
        wait_until("first edit not sent", || sets(&device) == 1);
        switched_to(&device, 3);
        let loaded = wait_for_load(&live);
        live.set(loaded.generation, 3, 0, 0, 0.6);
        drop(live);
        let selects = sent(&device).iter().filter(|r| matches!(r, Request::SelectBlock { .. })).count();
        assert_eq!(selects, 2);
    }

    #[test]
    fn a_block_the_pedal_selects_itself_is_left_before_the_next_edit() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(0, 3, 0, 0, 0.5);
        wait_until("first edit not sent", || sets(&device) == 1);
        let block = Value::Map(vec![(98.into(), 5.into()), (26.into(), 0.into())]);
        device.notify(39, Value::Map(vec![(121.into(), 19.into()), (106.into(), block)]));
        std::thread::sleep(std::time::Duration::from_millis(100));
        live.set(0, 3, 0, 0, 0.6);
        drop(live);
        let selects = sent(&device).iter().filter(|r| *r == &Request::SelectBlock { block: 3, path: 0 }).count();
        assert_eq!(selects, 2);
    }

    #[test]
    fn changes_and_loads_arrive_in_the_order_the_pedal_sent_them() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        param_changed(&device, 3, 1, 0.25);
        switched_to(&device, 3);
        let updates = updates_until_load(&live);
        let Update::Changed(before, change) = &updates[0] else { panic!("{updates:?}") };
        let Some(Update::Loaded(loaded)) = updates.last() else { panic!("{updates:?}") };
        assert_eq!(*change, ParamChange { slot: 3, sub: 0, param: Param::Model(1), value: Value::F32(0.25) });
        assert!(loaded.generation > *before, "a change from before the switch carries the old generation");
    }

    #[test]
    fn switching_snapshot_sends_op_88() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.select_snapshot(2);
        wait_until("op 88 not sent", || sent(&device).contains(&Request::SelectSnapshot { index: 2 }));
    }

    #[test]
    fn switching_a_block_sends_op_41() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set_enabled(0, 5, true);
        wait_until("op 41 not sent", || sent(&device).contains(&Request::SetBlockEnabled { block: 5, enabled: true }));
    }

    #[test]
    fn a_knob_turned_on_the_pedal_is_reported_without_reading_the_preset() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        param_changed(&device, 3, 1, 0.25);
        let mut updates = Vec::new();
        wait_until("no change reported", || {
            updates.extend(live.updates());
            !updates.is_empty()
        });
        assert!(matches!(updates[..], [Update::Changed(0, ParamChange { slot: 3, sub: 0, param: Param::Model(1), .. })]), "{updates:?}");
        assert!(sent(&device).is_empty(), "a reported change is not read back");
    }

    #[test]
    fn a_snapshot_or_block_change_on_the_pedal_reads_the_preset_again() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        device.notify(46, Value::Map(vec![(92.into(), 2.into())]));
        wait_for_load(&live);
        let block = Value::Map(vec![(98.into(), 5.into()), (59.into(), true.into())]);
        device.notify(49, Value::Map(vec![(121.into(), 17.into()), (106.into(), block)]));
        wait_for_load(&live);
    }

    #[test]
    fn nothing_more_is_sent_after_a_refusal_and_the_worker_says_it_stopped() {
        let device = device(|req| match req {
            Request::SetParam { .. } => Reply::Status(255, Value::Nil),
            _ => Reply::Ok(Value::Nil),
        });
        let live = Live::start(Session::connect(device.transport()).unwrap());
        assert!(live.running());
        live.set(0, 17, 0, 3, 0.45);
        wait_until("worker did not stop", || !live.running());
        live.set(0, 17, 0, 3, 1.8);
        live.set(0, 3, 0, 0, 0.5);
        drop(live);
        assert_eq!(sets(&device), 1);
    }

    #[test]
    fn a_refused_edit_is_reported() {
        let device = device(|req| match req {
            Request::SetParam { .. } => Reply::Status(5, Value::Nil),
            _ => Reply::Ok(Value::Nil),
        });
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set(0, 3, 0, 0, 0.5);
        let mut errors = Vec::new();
        wait_until("no error reported", || {
            errors.extend(live.errors());
            !errors.is_empty()
        });
        assert!(errors.iter().any(|e| e.contains("status 5")), "{errors:?}");
    }

    #[test]
    fn saving_sends_op_71_for_the_place_and_name_and_reports_it() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.save(0, 6, "NX Test");
        let mut saved = Vec::new();
        wait_until("save not reported", || {
            saved.extend(live.updates().into_iter().filter_map(|u| match u {
                Update::Saved(index) => Some(index),
                _ => None,
            }));
            !saved.is_empty()
        });
        assert_eq!(saved, [6]);
        assert!(sent(&device).contains(&Request::SavePreset { setlist: 0, index: 6, name: "NX Test".into() }));
    }

    #[test]
    fn a_loaded_preset_says_whether_the_pedal_holds_unsaved_edits() {
        let device = device(|req| match req {
            Request::PresetInfo => Reply::Ok(Value::Map(vec![(108.into(), 6.into()), (109.into(), "NX Test\0".into()), (117.into(), true.into())])),
            Request::PresetDocument => Reply::Ok(Value::from("document")),
            _ => echo(req),
        });
        let live = Live::start(Session::connect(device.transport()).unwrap());
        switched_to(&device, 6);
        assert!(wait_for_load(&live).preset.modified);
    }

    #[test]
    fn integers_booleans_and_extras_go_out_as_op_30_with_their_own_type() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.set_value(0, 3, 0, Param::Model(0), Value::from(9));
        live.set_value(0, 1, 0, Param::Extra(0), Value::Boolean(true));
        drop(live);
        let sent = sent(&device);
        assert!(sent.contains(&Request::SetParam { block: 3, path: 0, param: 0, value: Value::from(9) }));
        assert!(sent.contains(&Request::SetExtra { block: 1, path: 0, index: 0, value: Value::Boolean(true) }));
    }

    #[test]
    fn a_reported_trails_switch_is_passed_on_as_an_extra() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        let change = Value::Map(vec![
            (98.into(), 1.into()),
            (29.into(), false.into()),
            (26.into(), 0.into()),
            (28.into(), 0.into()),
            (119.into(), true.into()),
        ]);
        device.notify(30, Value::Map(vec![(82.into(), 0.into()), (106.into(), change)]));
        let mut changes = Vec::new();
        wait_until("no change passed on", || {
            changes.extend(live.updates().into_iter().filter_map(|u| match u {
                Update::Changed(_, change) => Some(change),
                _ => None,
            }));
            !changes.is_empty()
        });
        assert_eq!(changes, [ParamChange { slot: 1, sub: 0, param: Param::Extra(0), value: Value::Boolean(true) }]);
    }

    #[test]
    fn chain_edits_go_out_as_ops_39_40_and_28() {
        let device = device(echo);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.insert(0, 2, 80, 8);
        live.change_model(0, 2, 18);
        live.remove(0, 2);
        drop(live);
        let chain: Vec<Request> = sent(&device)
            .into_iter()
            .filter(|r| matches!(r, Request::InsertBlock { .. } | Request::ChangeModel { .. } | Request::RemoveBlock { .. }))
            .collect();
        assert_eq!(
            chain,
            [
                Request::InsertBlock { block: 2, model: 80, engine: 8, cab: None },
                Request::ChangeModel { block: 2, model: 18, cab: None },
                Request::RemoveBlock { block: 2 },
            ]
        );
    }

    #[test]
    fn chain_edits_meant_for_an_older_preset_are_dropped() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        switched_to(&device, 3);
        wait_for_load(&live);
        live.insert(0, 2, 80, 8);
        live.remove(0, 2);
        drop(live);
        assert!(sent(&device).iter().all(|r| !matches!(r, Request::InsertBlock { .. } | Request::RemoveBlock { .. })));
    }

    #[test]
    fn a_rewritten_document_goes_out_as_op_21_and_is_read_again_once_written() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.write_document(0, b"moved".to_vec());
        wait_until("op 21 not sent", || sent(&device).contains(&Request::WriteDocument { document: b"moved".to_vec() }));
        device.notify(21, Value::Nil);
        assert_eq!(wait_for_load(&live).preset.name, "Mark Lead");
    }

    #[test]
    fn a_document_meant_for_an_older_preset_is_not_written() {
        let device = pedal(NAMES);
        let live = Live::start(Session::connect(device.transport()).unwrap());
        switched_to(&device, 3);
        wait_for_load(&live);
        live.write_document(0, b"stale".to_vec());
        drop(live);
        assert!(sent(&device).iter().all(|r| !matches!(r, Request::WriteDocument { .. })));
    }

    #[test]
    fn assigning_writes_the_place_between_ops_255_and_254_and_reports_the_new_list() {
        let device = device(|req| match req {
            Request::ListPresets { .. } => Reply::Ok(Value::Array(vec![Value::Map(vec![(
                7.into(),
                Value::Map(vec![(109.into(), "Revv Home\0".into())]),
            )])])),
            _ => Reply::Ok(Value::Nil),
        });
        let live = Live::start(Session::connect(device.transport()).unwrap());
        live.assign(0, 7, "Revv Home", b"document".to_vec());
        let mut assigned = Vec::new();
        wait_until("assign not reported", || {
            assigned.extend(live.updates().into_iter().filter_map(|u| match u {
                Update::Assigned { index, presets } => Some((index, presets)),
                _ => None,
            }));
            !assigned.is_empty()
        });
        assert_eq!(assigned[0].0, 7);
        assert_eq!(assigned[0].1, [nx_proto::PresetEntry { index: 7, name: "Revv Home".into() }]);
        let control: Vec<Request> =
            device.requests().into_iter().filter(|(c, _, _)| *c == Channel::Control).map(|(_, _, r)| r).collect();
        let write = Request::WritePreset { setlist: 0, index: 7, name: "Revv Home".into(), document: b"document".to_vec() };
        assert_eq!(control, [Request::ControlBegin, write, Request::ControlStart, Request::ListPresets { setlist: 0 }]);
    }
}
