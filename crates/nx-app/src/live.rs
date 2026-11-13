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

use crate::device::{DevicePreset, preset_from_session};

/// Notifications after which the pedal's preset differs from what was read, so it is read again:
/// preset switched (4) [seen, 02], snapshot loaded (46) [seen, 05], a block changed: enabled,
/// model (49) [seen, 04, live-05].
const READ_AGAIN: [u64; 3] = [4, 46, 49];
/// A preset switch is under way (8), before 4 [seen, 02]; addresses are stale from here on.
const SWITCHING: u64 = 8;
/// A parameter changed, by nx-edit or on the pedal: `{106: {98, 29, 26, 28, 119}}` like op 30 [seen,
/// live-05].
const PARAM_CHANGED: u64 = 30;
/// A block was selected, by op 78 or on the pedal: `{106: {98, 26}}` [seen, 04, live-05].
const BLOCK_SELECTED: u64 = 39;
/// How often the worker looks at notifications while no command comes in.
const POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Edit {
    generation: u64,
    slot: u64,
    sub: u64,
    param: u64,
    value: f32,
}

enum Command {
    Edit(Edit),
    Select { setlist: u64, index: u64 },
    Snapshot(u64),
    Enabled { generation: u64, slot: u64, enabled: bool },
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

    /// Turns a block on or off (op 41) on the preset of `generation`; `slot` as op 30 addresses it.
    pub fn set_enabled(&self, generation: u64, slot: u64, enabled: bool) {
        self.send(Command::Enabled { generation, slot, enabled });
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
            Command::Edit(_) | Command::Enabled { .. } => Ok(()),
            Command::Select { setlist, index } => {
                let outcome = self.session.request(Request::SelectPreset { setlist, index }).map(drop);
                self.next_generation();
                outcome
            }
            Command::Snapshot(index) => self.session.request(Request::SelectSnapshot { index }).map(drop),
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
                Update::Changed(..) => None,
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
        assert_eq!(*change, ParamChange { slot: 3, sub: 0, param: 1, value: 0.25 });
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
        assert!(matches!(updates[..], [Update::Changed(0, ParamChange { slot: 3, sub: 0, param: 1, .. })]), "{updates:?}");
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
}
