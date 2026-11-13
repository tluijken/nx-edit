//! Undo and redo of knob turns on the pedal's preset. A turn is one drag, from where the knob was
//! when it was grabbed to where it was let go.

#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub block: String,
    pub param: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Debug, Default)]
pub struct History {
    done: Vec<Turn>,
    undone: Vec<Turn>,
}

impl History {
    pub fn push(&mut self, turn: Turn) {
        if (turn.before - turn.after).abs() < 1e-9 {
            return;
        }
        self.done.push(turn);
        self.undone.clear();
    }

    /// The latest turn to take back; set its parameter to `before`.
    pub fn undo(&mut self) -> Option<Turn> {
        let turn = self.done.pop()?;
        self.undone.push(turn.clone());
        Some(turn)
    }

    /// The latest undone turn to make again; set its parameter to `after`.
    pub fn redo(&mut self) -> Option<Turn> {
        let turn = self.undone.pop()?;
        self.done.push(turn.clone());
        Some(turn)
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

    #[test]
    fn undo_gives_back_turns_newest_first_and_redo_replays_them() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.6));
        history.push(turn("Bass", 0.3, 0.4));
        assert_eq!(history.undo(), Some(turn("Bass", 0.3, 0.4)));
        assert_eq!(history.undo(), Some(turn("Drive", 0.45, 0.6)));
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), Some(turn("Drive", 0.45, 0.6)));
        assert_eq!(history.redo(), Some(turn("Bass", 0.3, 0.4)));
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn a_new_turn_drops_what_was_undone() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.6));
        history.undo();
        history.push(turn("Bass", 0.3, 0.4));
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn a_drag_that_ends_where_it_started_is_not_kept() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.45));
        assert_eq!(history.undo(), None);
    }

    #[test]
    fn clearing_forgets_both_ways() {
        let mut history = History::default();
        history.push(turn("Drive", 0.45, 0.6));
        history.push(turn("Bass", 0.3, 0.4));
        history.undo();
        history.clear();
        assert_eq!((history.undo(), history.redo()), (None, None));
    }
}
