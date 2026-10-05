//! Undo, redo, and showing what a change would do before it does it.
//!
//! Snapshots, not inverse edits. The CST throws its edit list away the moment
//! it renders — `edit_file` builds a throwaway `Doc`, renders, and keeps only
//! the resulting `String` — so there is no edit log to invert, and building one
//! would mean maintaining the invariant that inverting step *n* restores the
//! state before it, across a re-parse. Config files are small; a hundred levels
//! of a 40 KB file is 4 MB. Snapshots are simply the right trade here.
//!
//! Undo is a workspace operation, not a per-file one: a merge can touch two
//! files, and an undo that restored only one of them would be worse than none.

use serde::Serialize;

/// The state of every file a single action touched, before it touched them.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// What the action was, in words: "merged 3 set blocks"
    pub label: String,
    /// (absolute path, the text it had before)
    pub files: Vec<(String, String)>,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

/// Deep enough that nobody hits the end during a session; small enough that
/// the memory never matters.
const DEPTH: usize = 200;

impl History {
    pub fn record(&mut self, label: impl Into<String>, files: Vec<(String, String)>) {
        self.undo.push(Snapshot { label: label.into(), files });
        if self.undo.len() > DEPTH {
            self.undo.remove(0);
        }
        // A new action makes the redo branch unreachable.
        self.redo.clear();
    }

    /// What undo would undo, for the button's label.
    pub fn next_undo(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }
    pub fn next_redo(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Take the last action off the undo stack. The caller restores the files
    /// and hands back what they were, which becomes the redo entry.
    pub fn pop_undo(&mut self) -> Option<Snapshot> {
        self.undo.pop()
    }
    pub fn push_redo(&mut self, s: Snapshot) {
        self.redo.push(s);
    }
    pub fn pop_redo(&mut self) -> Option<Snapshot> {
        self.redo.pop()
    }
    pub fn push_undo(&mut self, s: Snapshot) {
        self.undo.push(s);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Change {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub change: Change,
    /// Line number in the old text, where there is one
    pub old_line: Option<usize>,
    /// Line number in the new text, where there is one
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
    /// True when the change is too large to show line by line
    pub truncated: bool,
}

/// A line diff good enough to answer "is this what I meant?".
///
/// Trims the common head and tail, then shows what is left with a few lines of
/// context. Not a minimal diff — for a config edit the changed region is
/// usually contiguous, and an honest big block beats a clever small one.
pub fn diff(old: &str, new: &str, context: usize) -> Diff {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();

    let mut head = 0;
    while head < a.len() && head < b.len() && a[head] == b[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head && tail < b.len() - head
        && a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
    {
        tail += 1;
    }

    let removed_span = head..(a.len() - tail);
    let added_span = head..(b.len() - tail);
    let removed = removed_span.len();
    let added = added_span.len();

    let mut lines = Vec::new();
    let truncated = removed + added > 400;

    let ctx_from = head.saturating_sub(context);
    for i in ctx_from..head {
        lines.push(DiffLine {
            change: Change::Same,
            old_line: Some(i + 1),
            new_line: Some(i + 1),
            text: a[i].to_string(),
        });
    }

    if truncated {
        lines.push(DiffLine {
            change: Change::Same,
            old_line: None,
            new_line: None,
            text: format!("… {removed} lines replaced by {added} …"),
        });
    } else {
        for i in removed_span.clone() {
            lines.push(DiffLine {
                change: Change::Removed,
                old_line: Some(i + 1),
                new_line: None,
                text: a[i].to_string(),
            });
        }
        for i in added_span.clone() {
            lines.push(DiffLine {
                change: Change::Added,
                old_line: None,
                new_line: Some(i + 1),
                text: b[i].to_string(),
            });
        }
    }

    let after_end = (a.len() - tail + context).min(a.len());
    for i in (a.len() - tail)..after_end {
        lines.push(DiffLine {
            change: Change::Same,
            old_line: Some(i + 1),
            new_line: Some(i + 1 + b.len() - a.len()),
            text: a[i].to_string(),
        });
    }

    Diff { lines, added, removed, truncated }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_change_shows_nothing() {
        let d = diff("a\nb\nc\n", "a\nb\nc\n", 2);
        assert_eq!(d.added, 0);
        assert_eq!(d.removed, 0);
        assert!(d.lines.iter().all(|l| l.change == Change::Same));
    }

    #[test]
    fn a_changed_line_shows_both_sides() {
        let d = diff("a\nb\nc\n", "a\nB\nc\n", 1);
        assert_eq!(d.removed, 1);
        assert_eq!(d.added, 1);
        assert!(d.lines.iter().any(|l| l.change == Change::Removed && l.text == "b"));
        assert!(d.lines.iter().any(|l| l.change == Change::Added && l.text == "B"));
        // and the unchanged neighbours are there for context
        assert!(d.lines.iter().any(|l| l.change == Change::Same && l.text == "a"));
        assert!(d.lines.iter().any(|l| l.change == Change::Same && l.text == "c"));
    }

    #[test]
    fn an_insertion_removes_nothing() {
        let d = diff("a\nc\n", "a\nb\nc\n", 1);
        assert_eq!(d.removed, 0);
        assert_eq!(d.added, 1);
    }

    #[test]
    fn a_deletion_adds_nothing() {
        let d = diff("a\nb\nc\n", "a\nc\n", 1);
        assert_eq!(d.removed, 1);
        assert_eq!(d.added, 0);
    }

    #[test]
    fn a_huge_change_is_summarised_rather_than_listed() {
        let old: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let new: String = (0..500).map(|i| format!("other {i}\n")).collect();
        let d = diff(&old, &new, 2);
        assert!(d.truncated);
        assert!(d.lines.len() < 20);
    }

    #[test]
    fn history_records_and_unwinds() {
        let mut h = History::default();
        assert!(h.next_undo().is_none());
        h.record("changed sendq", vec![("a.conf".into(), "before".into())]);
        assert_eq!(h.next_undo(), Some("changed sendq"));
        let s = h.pop_undo().unwrap();
        assert_eq!(s.files[0].1, "before");
        h.push_redo(s);
        assert_eq!(h.next_redo(), Some("changed sendq"));
    }

    #[test]
    fn a_new_action_discards_the_redo_branch() {
        let mut h = History::default();
        h.record("one", vec![]);
        let s = h.pop_undo().unwrap();
        h.push_redo(s);
        assert!(h.next_redo().is_some());
        h.record("two", vec![]);
        assert!(h.next_redo().is_none(), "redo must not survive a new action");
    }

    #[test]
    fn the_stack_is_capped() {
        let mut h = History::default();
        for i in 0..(DEPTH + 50) {
            h.record(format!("edit {i}"), vec![]);
        }
        assert_eq!(h.undo.len(), DEPTH);
        assert_eq!(h.next_undo(), Some(format!("edit {}", DEPTH + 49).as_str()));
    }
}
