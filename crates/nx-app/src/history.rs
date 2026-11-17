//! Undo and redo of edits to the preset shown: knob turns, and chain edits on the pedal. A turn is
//! one drag, from where the knob was when it was grabbed to where it was let go. A chain edit is
//! undone by writing back the pedal's document from before it, so redoing writes the one undo
//! replaced.

#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub block: String,
    pub param: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Turn(Turn),
    /// The pedal's document to write back.
    Document(Vec<u8>),
}

impl Edit {
    /// The same edit for the other stack: a document undone or redone is replaced by the one it
    /// overwrites.
    fn reversed(&self, now: &[u8]) -> Edit {
        match self {
            Edit::Turn(turn) => Edit::Turn(turn.clone()),
            Edit::Document(_) => Edit::Document(now.to_vec()),
        }
    }
}

#[derive(Debug, Default)]
pub struct History {
    done: Vec<Edit>,
    undone: Vec<Edit>,
}

impl History {
    pub fn push(&mut self, turn: Turn) {
        if (turn.before - turn.after).abs() < 1e-9 {
            return;
        }
        self.done.push(Edit::Turn(turn));
        self.undone.clear();
    }

    /// A chain edit about to be made on the pedal, with its document from before.
    pub fn push_document(&mut self, before: Vec<u8>) {
        self.done.push(Edit::Document(before));
        self.undone.clear();
    }

    /// The latest edit to take back: set a turn's parameter to `before`, write a document. `now`
    /// is the pedal's document at this moment, for redo.
    pub fn undo(&mut self, now: &[u8]) -> Option<Edit> {
        let edit = self.done.pop()?;
        self.undone.push(edit.reversed(now));
        Some(edit)
    }

    /// The latest undone edit to make again: set a turn's parameter to `after`, write a document.
    pub fn redo(&mut self, now: &[u8]) -> Option<Edit> {
        let edit = self.undone.pop()?;
        self.done.push(edit.reversed(now));
        Some(edit)
    }

    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(param: &str, before: f64, after: f64) -> Turn {
        Turn { block: "block2".into(), param: param.into(), before, after }
    }

    fn turned(param: &str, before: f64, after: f64) -> Option<Edit> {
        Some(Edit::Turn(turn(param, before, after)))
    }

    #[test]
    fn undo_gives_back_turns_newest_first_and_redo_replays_them() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.6));
        history.push(turn("Bass", 0.3, 0.4));
        assert_eq!(history.undo(&[]), turned("Bass", 0.3, 0.4));
        assert_eq!(history.undo(&[]), turned("Drive", 0.45, 0.6));
        assert_eq!(history.undo(&[]), None);
        assert_eq!(history.redo(&[]), turned("Drive", 0.45, 0.6));
        assert_eq!(history.redo(&[]), turned("Bass", 0.3, 0.4));
        assert_eq!(history.redo(&[]), None);
    }

    #[test]
    fn a_chain_edit_is_undone_by_its_old_document_and_redone_by_the_one_undo_replaced() {
        let mut history = History::default();
        history.push_document(b"before".to_vec());
        assert_eq!(history.undo(b"after"), Some(Edit::Document(b"before".to_vec())));
        assert_eq!(history.redo(b"before"), Some(Edit::Document(b"after".to_vec())));
        assert_eq!(history.undo(b"after"), Some(Edit::Document(b"before".to_vec())));
    }

    #[test]
    fn a_new_edit_drops_what_was_undone() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.6));
        history.undo(&[]);
        history.push(turn("Bass", 0.3, 0.4));
        assert_eq!(history.redo(&[]), None);
        history.undo(&[]);
        history.push_document(Vec::new());
        assert_eq!(history.redo(&[]), None);
    }

    #[test]
    fn a_drag_that_ends_where_it_started_is_not_kept() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.45));
        assert_eq!(history.undo(&[]), None);
    }

    #[test]
    fn clearing_forgets_both_ways() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.6));
        history.push(turn("Bass", 0.3, 0.4));
        history.undo(&[]);
        history.clear();
        assert_eq!((history.undo(&[]), history.redo(&[])), (None, None));
    }
}
