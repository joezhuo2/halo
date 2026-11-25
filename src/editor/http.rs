use super::Editors;
use super::layout::Dir;
use crate::http_client::{Request, Response};
use std::path::Path;

impl Editors {
    pub(super) fn send_http(&mut self, req: Request, ctx: &eframe::egui::Context) {
        let waiting = format!("Waiting for {} {} ...\n\nRun Rest Client: Cancel Request to stop it.\n", req.method, req.url);
        self.http.send(req, ctx);
        self.show_response(Response { title: "Response".into(), text: waiting, syntax: "response.txt" });
    }

    pub(super) fn after_http(&mut self) {
        if let Some(response) = self.http.poll() {
            self.show_response(response);
        }
    }

    fn show_response(&mut self, response: Response) {
        let existing = self.docs.values_mut().find(|d| d.buf.read_only && d.buf.path.is_none() && d.buf.title.starts_with("Response"));
        if let Some(doc) = existing {
            doc.buf.title.clone_from(&response.title);
        }
        let g = self.active_tab().map_or(self.active, |(g, _)| g);
        let shown = self.groups.iter().position(|group| group.tabs.iter().any(|t| self.docs.get(&t.doc).is_some_and(|d| d.buf.read_only && d.buf.title == response.title)));
        self.active = shown.unwrap_or_else(|| self.neighbor(g, Dir::Right).unwrap_or_else(|| self.split_group(g, Dir::Right)));
        self.open_text(&response.title, &response.text, Some(Path::new(response.syntax)));
    }
}
