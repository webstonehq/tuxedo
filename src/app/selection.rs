use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct Selection {
    selected: HashSet<usize>,
    editing: Option<usize>,
    edit_stale: bool,
}

impl Selection {
    pub fn is_selected(&self, abs: usize) -> bool {
        self.selected.contains(&abs)
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected.iter().copied()
    }

    pub fn toggle(&mut self, abs: usize) {
        if !self.selected.insert(abs) {
            self.selected.remove(&abs);
        }
    }

    pub fn clear(&mut self) {
        self.selected.clear();
    }

    pub fn remap_swaps(&mut self, swaps: &[(usize, usize)]) {
        for &(a, b) in swaps {
            let a_selected = self.selected.remove(&a);
            let b_selected = self.selected.remove(&b);
            if a_selected {
                self.selected.insert(b);
            }
            if b_selected {
                self.selected.insert(a);
            }
        }
    }

    pub fn editing(&self) -> Option<usize> {
        self.editing
    }

    /// Enter edit mode on `abs`. Drops the multi-select set — editing one task
    /// while it's also flagged for bulk operations is structurally incoherent
    /// (`complete_selected` would double-handle the editing index).
    pub fn enter_edit(&mut self, abs: usize) {
        self.editing = Some(abs);
        self.edit_stale = false;
        self.selected.clear();
    }

    /// A disk reload invalidates row identity, while the draft stays recoverable.
    pub fn invalidate_edit(&mut self) {
        self.edit_stale = self.editing.is_some();
    }

    pub fn edit_stale(&self) -> bool {
        self.edit_stale
    }

    pub fn exit_edit(&mut self) {
        self.editing = None;
        self.edit_stale = false;
    }
}
