use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct Selection {
    selected: HashSet<usize>,
    editing: Option<usize>,
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

    pub fn retain(&mut self, mut keep: impl FnMut(usize) -> bool) -> usize {
        let before = self.selected.len();
        self.selected.retain(|i| keep(*i));
        before - self.selected.len()
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

    /// Preserve task identity when storage removes lines or inserts a
    /// recurring successor. Display positions never enter these mappings.
    pub fn remove_indices(&mut self, removed: &[usize]) {
        let remap = |i: usize| {
            (!removed.contains(&i)).then(|| i - removed.iter().filter(|&&r| r < i).count())
        };
        self.selected = self.selected.iter().filter_map(|&i| remap(i)).collect();
        self.editing = self.editing.and_then(remap);
    }

    pub fn insert_index(&mut self, at: usize) {
        self.selected = self
            .selected
            .iter()
            .map(|&i| if i >= at { i + 1 } else { i })
            .collect();
        self.editing = self.editing.map(|i| if i >= at { i + 1 } else { i });
    }

    pub fn editing(&self) -> Option<usize> {
        self.editing
    }

    /// Enter edit mode on `abs`. Drops the multi-select set — editing one task
    /// while it's also flagged for bulk operations is structurally incoherent
    /// (`complete_selected` would double-handle the editing index).
    pub fn enter_edit(&mut self, abs: usize) {
        self.editing = Some(abs);
        self.selected.clear();
    }

    pub fn exit_edit(&mut self) {
        self.editing = None;
    }
}
