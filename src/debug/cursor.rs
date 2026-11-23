use super::*;

#[derive(Clone, Copy, Default, PartialEq)]
pub struct CursorCommands {
    run: bool,
    jump: bool,
}

impl CursorCommands {
    pub fn allows(self, cmd: &Command) -> bool {
        match cmd {
            Command::RunToCursor => self.run,
            Command::JumpToCursor => self.jump,
            _ => true,
        }
    }
}

fn shorten(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut out = String::new();
    for c in chars.by_ref().take(limit) {
        if matches!(c, '\r' | '\n') {
            out.push('…');
            return out;
        }
        out.push(c);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
}

impl Marks {
    pub fn inline_label(&self, text: &str) -> String {
        let mut seen = HashSet::new();
        let mut label = String::new();
        for word in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$') {
            if let Some(value) = self.inline.get(word) && seen.insert(word) {
                if !label.is_empty() {
                    label.push_str(", ");
                }
                label.push_str(word);
                label.push_str(" = ");
                label.push_str(value);
                if label.len() >= 120 && label.chars().count() >= 120 {
                    break;
                }
            }
        }
        shorten(&label, 120)
    }
}

pub(super) fn with_cursor(mut args: Value, line: usize) -> (Value, usize) {
    let sent = args["breakpoints"].as_array_mut().unwrap();
    let index = sent.iter().position(|b| b["line"] == line).unwrap_or(sent.len());
    let point = json!({"line": line});
    if index == sent.len() {
        sent.push(point);
    } else {
        sent[index] = point;
    }
    (args, index)
}

pub(super) fn first_target(body: &Value) -> Option<i64> {
    body["targets"][0]["id"].as_i64()
}

impl Live {
    pub(super) fn moving(&self) -> bool {
        self.wants.values().any(|w| matches!(w, Want::Run | Want::Goto | Want::GotoTargets(_) | Want::RunTo(..) | Want::Restart))
    }

    pub(super) fn cursor_commands(&self) -> CursorCommands {
        let run = self.state == State::Stopped && self.thread.is_some() && self.stopping == 0 && !self.moving();
        CursorCommands { run, jump: run && self.can("supportsGotoTargetsRequest") }
    }

    pub(super) fn start_cursor_command(&mut self, cmd: &Command, path: &Path, line: usize, marks: &Marks) {
        let Some(thread) = self.thread.filter(|_| self.cursor_commands().allows(cmd)) else { return };
        if *cmd == Command::RunToCursor {
            let (args, index) = with_cursor(source_breakpoints(path, marks.of(path)), line);
            self.temporary = Some((path.to_owned(), line));
            self.ask("setBreakpoints", args, Want::RunTo(thread, index));
        } else {
            self.ask("gotoTargets", json!({"source": {"path": path}, "line": line}), Want::GotoTargets(thread));
        }
    }

    pub(super) fn clear_temporary(&mut self, marks: &Marks) {
        self.wants.retain(|_, w| !matches!(w, Want::RunTo(..) | Want::GotoTargets(_) | Want::Goto));
        if let Some((path, _)) = self.temporary.take() {
            self.send_breakpoints(&path, marks.of(&path));
        }
    }

    pub(super) fn refresh_inline(&self, marks: &mut Marks) {
        marks.inline.clear();
        for scope in self.scopes.iter().filter(|s| !s.expensive) {
            for var in self.children.get(&scope.reference).into_iter().flatten() {
                if !var.name.is_empty() {
                    marks.inline.entry(var.name.clone()).or_insert_with(|| shorten(&var.value, 60));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_breakpoints_replace_conditions_without_changing_saved_points() {
        let base = json!({"breakpoints": [{"line": 3, "condition": "x > 0"}, {"line": 9, "logMessage": "x"}]});
        let (args, index) = with_cursor(base.clone(), 3);
        assert_eq!(index, 0);
        assert_eq!(args["breakpoints"], json!([{"line": 3}, {"line": 9, "logMessage": "x"}]));
        let (args, index) = with_cursor(base.clone(), 8);
        assert_eq!(index, 2);
        assert_eq!(args["breakpoints"][index], json!({"line": 8}));
        assert_eq!(args["breakpoints"][0], base["breakpoints"][0]);
    }

    #[test]
    fn takes_the_first_goto_target_id() {
        assert_eq!(first_target(&json!({"targets": [{"id": 0}, {"id": 10}]})), Some(0));
        assert_eq!(first_target(&json!({"targets": []})), None);
        assert_eq!(first_target(&json!({"targets": [{"label": "missing id"}]})), None);
    }

    #[test]
    fn headless_editor_paints_values_only_in_the_selected_frame_window() {
        use crate::{editor::Editors, git::Git};
        let path = std::env::temp_dir().join(format!("halo-inline-values-{}.txt", std::process::id()));
        std::fs::write(&path, "x\n".repeat(65)).unwrap();
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.open(&path);
        editors.marks.inline.insert("x".into(), "42".into());
        editors.marks.stopped = Some((path.clone(), 52));
        let drawn = |editors: &mut Editors, enabled| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 1800.0))), ..Default::default() };
            let settings = Settings { inline_values: enabled, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &Theme::dark(), &settings, &Git::new(), &mut vec![]));
            out.textures_delta.clear();
            fn count(shape: &egui::Shape) -> usize {
                match shape {
                    egui::Shape::Vec(list) => list.iter().map(count).sum(),
                    egui::Shape::Text(t) => usize::from(t.galley.text() == "x = 42"),
                    _ => 0,
                }
            }
            out.shapes.iter().map(|s| count(&s.shape)).sum::<usize>()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while editors.status().is_none() {
            assert!(std::time::Instant::now() < deadline);
            drawn(&mut editors, true);
        }
        assert_eq!(drawn(&mut editors, true), 51);
        assert_eq!(drawn(&mut editors, false), 0);
        editors.marks.stopped = Some((PathBuf::from("different.txt"), 52));
        assert_eq!(drawn(&mut editors, true), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn inline_labels_match_whole_identifiers_once_in_source_order() {
        let mut marks = Marks::default();
        marks.inline.extend([("x".into(), "1".into()), ("xx".into(), "2".into()), ("名字".into(), "3".into())]);
        assert_eq!(marks.inline_label("x1 _x xx x + xx + 名字"), "xx = 2, x = 1, 名字 = 3");
        assert_eq!(shorten("a\nb", 5), "a…");
        assert_eq!(shorten("日本語", 2), "日…");
    }
}
