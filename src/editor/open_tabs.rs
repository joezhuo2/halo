use super::Editors;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct OpenTab {
    pub group: usize,
    pub index: usize,
    pub title: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub active: bool,
    pub severity: Option<u8>,
}

impl Editors {
    pub fn severities(&self) -> BTreeMap<String, u8> {
        if self.config.diagnostics { self.lsp.severities() } else { BTreeMap::new() }
    }

    pub fn open_tabs(&self) -> Vec<Vec<OpenTab>> {
        let focused = self.active.min(self.groups.len().saturating_sub(1));
        let severities = self.severities();
        self.groups
            .iter()
            .enumerate()
            .map(|(g, group)| {
                group
                    .tabs
                    .iter()
                    .enumerate()
                    .filter_map(|(t, tab)| {
                        let doc = self.docs.get(&tab.doc)?;
                        let active = g == focused && t == group.active;
                        let severity = doc.buf.path.as_deref().and_then(|p| crate::lsp::worst(&severities, p));
                        Some(OpenTab { group: g, index: t, title: doc.buf.title.clone(), path: doc.buf.path.clone(), dirty: doc.buf.is_dirty(), active, severity })
                    })
                    .collect()
            })
            .collect()
    }

    pub fn show_tab(&mut self, g: usize, t: usize) {
        if self.groups.get(g).is_some_and(|group| t < group.tabs.len()) {
            self.groups[g].active = t;
            self.active = g;
            self.want_focus = true;
        }
    }

    pub fn close_open_tab(&mut self, g: usize, t: usize) {
        self.request_close(g, t);
    }

    pub fn close_all_tabs(&mut self) {
        for g in (0..self.groups.len()).rev() {
            for t in (0..self.groups[g].tabs.len()).rev() {
                let doc = self.groups[g].tabs[t].doc;
                if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                    self.close_tab(g, t);
                }
            }
        }
        let left = self.groups.iter().position(|group| !group.tabs.is_empty());
        if let (None, Some(g)) = (self.closing, left) {
            self.closing = Some((g, self.groups[g].active.min(self.groups[g].tabs.len() - 1)));
        }
    }
}
