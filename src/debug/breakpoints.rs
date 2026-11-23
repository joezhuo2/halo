use super::*;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FunctionBreakpoint {
    name: String,
    enabled: bool,
    condition: String,
    hit: String,
}

impl FunctionBreakpoint {
    fn new(name: &str) -> Self {
        Self { name: name.into(), enabled: true, condition: String::new(), hit: String::new() }
    }

    fn request(&self, caps: &Value) -> Value {
        let mut point = json!({"name": self.name});
        for (key, capability, text) in [
            ("condition", "supportsConditionalBreakpoints", &self.condition),
            ("hitCondition", "supportsHitConditionalBreakpoints", &self.hit),
        ] {
            if caps[capability] == true && !text.trim().is_empty() {
                point[key] = json!(text.trim());
            }
        }
        point
    }
}

#[derive(Default, Debug, PartialEq)]
pub(super) struct Verification {
    id: Option<i64>,
    verified: bool,
    message: String,
}

pub(super) fn verification(keys: Vec<String>, body: &Value) -> HashMap<String, Verification> {
    keys.into_iter().zip(body["breakpoints"].as_array().into_iter().flatten()).map(|(key, b)| {
        (key, Verification {
            id: b["id"].as_i64(),
            verified: b["verified"] == true,
            message: b["message"].as_str().unwrap_or_default().into(),
        })
    }).collect()
}

#[derive(Debug, PartialEq)]
pub(super) struct DataBreakpoint {
    pub(super) id: String,
    name: String,
    access: Option<String>,
    enabled: bool,
    pub(super) status: Verification,
}

impl DataBreakpoint {
    pub(super) fn from_info(name: String, body: &Value) -> Result<Self, String> {
        let id = body["dataId"].as_str().ok_or_else(|| body["description"].as_str()
            .unwrap_or("The adapter cannot set a data breakpoint on this variable.").to_owned())?;
        let types = body["accessTypes"].as_array();
        let access = ["write", "readWrite", "read"].into_iter().find(|t| types.is_some_and(|v| v.iter().any(|a| a == t)));
        Ok(Self { id: id.into(), name, access: access.map(str::to_owned), enabled: true, status: Verification::default() })
    }

    fn request(&self) -> Value {
        let mut point = json!({"dataId": self.id});
        if let Some(access) = &self.access {
            point["accessType"] = json!(access);
        }
        point
    }
}

impl Live {
    pub(super) fn send_functions(&mut self, points: &[FunctionBreakpoint]) {
        if !self.can("supportsFunctionBreakpoints") { return; }
        self.wants.retain(|_, w| !matches!(w, Want::Functions(_)));
        self.function_status.clear();
        let enabled = points.iter().filter(|b| b.enabled);
        let names = enabled.clone().map(|b| b.name.clone()).collect();
        let items: Vec<_> = enabled.map(|b| b.request(&self.caps)).collect();
        self.ask("setFunctionBreakpoints", json!({"breakpoints": items}), Want::Functions(names));
    }

    pub(super) fn data_info(&mut self, parent: i64, name: String) {
        if self.state == State::Stopped && self.can("supportsDataBreakpoints") {
            self.ask("dataBreakpointInfo", json!({"variablesReference": parent, "name": name}), Want::DataInfo(name));
        }
    }

    pub(super) fn add_data(&mut self, point: DataBreakpoint) {
        if let Some(existing) = self.data.iter_mut().find(|b| b.id == point.id) {
            if existing.enabled { return; }
            existing.enabled = true;
        } else {
            self.data.push(point);
        }
        self.send_data();
    }

    pub(super) fn remove_data(&mut self, id: &str) {
        self.data.retain(|b| b.id != id);
        self.send_data();
    }

    pub(super) fn send_data(&mut self) {
        if !self.can("supportsDataBreakpoints") { return; }
        self.wants.retain(|_, w| !matches!(w, Want::Data(_)));
        let mut ids = vec![];
        let mut items = vec![];
        for point in &mut self.data {
            point.status = Verification::default();
            if point.enabled {
                ids.push(point.id.clone());
                items.push(point.request());
            }
        }
        self.ask("setDataBreakpoints", json!({"breakpoints": items}), Want::Data(ids));
    }

    pub(super) fn update_breakpoint(&mut self, found: &Value) -> bool {
        let Some(id) = found["id"].as_i64() else { return false };
        let mut states = self.function_status.values_mut().chain(self.data.iter_mut().map(|b| &mut b.status));
        let Some(state) = states.find(|s| s.id == Some(id)) else { return false };
        state.verified = found["verified"] == true;
        state.message = found["message"].as_str().unwrap_or_default().into();
        true
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
pub struct BreakpointCommands {
    functions: bool,
    data: bool,
}

impl BreakpointCommands {
    pub fn allows(self, cmd: &Command) -> bool {
        match cmd {
            Command::AddFunctionBreakpoint => self.functions,
            Command::BreakOnValueChange => self.data,
            _ => true,
        }
    }
}

impl Debug {
    pub fn breakpoint_commands(&self, folder: bool) -> BreakpointCommands {
        BreakpointCommands {
            functions: folder && (self.live.is_empty() || self.live.iter().any(|l| l.can("supportsFunctionBreakpoints"))),
            data: self.live.get(self.focus).is_some_and(|l| l.state == State::Stopped && l.can("supportsDataBreakpoints")),
        }
    }

    pub(super) fn breakpoint_count(&self, marks: &Marks) -> usize {
        marks.functions.len() + marks.files.iter().map(|f| f.1.len()).sum::<usize>() + self.live.iter().map(|l| l.data.len()).sum::<usize>()
    }

    pub(super) fn confirm_breakpoint_removal(&mut self, ctx: &egui::Context, marks: &mut Marks) {
        if !self.confirm_clear { return }
        let count = self.breakpoint_count(marks);
        let mut choice = None;
        let modal = ui::modal(ctx, "confirm_clear_breakpoints", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(340.0);
            ui.heading("Remove all breakpoints?");
            ui.label(format!("This removes {count} breakpoints."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if keys.primary(ui, "Remove All", true) { choice = Some(true); }
                if ui.button("Cancel").clicked() { choice = Some(false); }
            });
        });
        if let Some(remove) = choice.or_else(|| modal.should_close().then_some(false)) {
            self.confirm_clear = false;
            if remove {
                marks.clear();
                for live in &mut self.live {
                    live.wants.retain(|_, w| !matches!(w, Want::DataInfo(_)));
                    if !live.data.is_empty() {
                        live.data.clear();
                        live.send_data();
                    }
                }
            }
        }
    }

    pub(super) fn additional_breakpoints_ui(&mut self, ui: &mut Ui, theme: &Theme, marks: &mut Marks) {
        let functions = self.breakpoint_commands(true).functions;
        if let Some(Editing::FunctionName(text)) = &mut self.editing {
            if functions {
                if let Some(ok) = inline_edit(ui, text, "Function name") {
                    let name = text.trim();
                    if ok && !name.is_empty() {
                        if marks.functions.iter().any(|b| b.name == name) {
                            ui::notify(format!("A breakpoint already exists for {name}."));
                        } else {
                            marks.functions.push(FunctionBreakpoint::new(name));
                            marks.functions_changed = true;
                        }
                    }
                    self.editing = None;
                }
            } else {
                self.editing = None;
            }
        }
        let supports = |cap| self.live.iter().any(|l| l.can("supportsFunctionBreakpoints") && l.can(cap));
        let condition = supports("supportsConditionalBreakpoints");
        let hit = supports("supportsHitConditionalBreakpoints");
        let session = self.live.get(self.focus).filter(|l| l.can("supportsFunctionBreakpoints"));
        let pending = Verification::default();
        let mut removed = None;
        for (i, point) in marks.functions.iter_mut().enumerate() {
            let tail = 24.0 + 22.0 * (u8::from(condition) + u8::from(hit)) as f32;
            let detail = [&point.condition, &point.hit].into_iter().filter(|s| !s.is_empty()).map(String::as_str).collect::<Vec<_>>().join("  ");
            let status = session.filter(|_| point.enabled).map(|l| l.function_status.get(&point.name).unwrap_or(&pending));
            let (r, changed, hot) = point_row(ui, theme, (&point.name, &detail), &mut point.enabled, status, tail, functions);
            marks.functions_changed |= changed;
            if hot && hover_button(ui, theme, r, 0, icons::X, "Remove Function Breakpoint") {
                removed = Some(i);
            }
            if hot && condition && hover_button(ui, theme, r, 1, icons::PENCIL_SIMPLE, "Edit Condition") {
                self.editing = Some(Editing::FunctionCondition(point.name.clone(), Ask::Condition, point.condition.clone()));
            }
            if hot && hit && hover_button(ui, theme, r, 1 + usize::from(condition), icons::HASH, "Edit Hit Count") {
                self.editing = Some(Editing::FunctionCondition(point.name.clone(), Ask::Hit, point.hit.clone()));
            }
            if let Some(Editing::FunctionCondition(name, ask, text)) = &mut self.editing && name == &point.name {
                let is_hit = *ask == Ask::Hit;
                if let Some(ok) = inline_edit(ui, text, if is_hit { "Hit count" } else { "Condition" }) {
                    if ok && (if is_hit { hit } else { condition }) {
                        *(if is_hit { &mut point.hit } else { &mut point.condition }) = text.trim().to_owned();
                        marks.functions_changed = true;
                    }
                    self.editing = None;
                }
            }
        }
        if let Some(i) = removed {
            marks.functions.remove(i);
            marks.functions_changed = true;
        }
        let multiple = self.live.len() > 1;
        for live in &mut self.live {
            let mut changed = false;
            let mut removed = None;
            for point in &mut live.data {
                let access = point.access.as_deref().unwrap_or("adapter default");
                let detail = if multiple { format!("{access} · {}", live.config.name) } else { access.into() };
                let status = point.enabled.then_some(&point.status);
                let (r, toggled, hot) = point_row(ui, theme, (&point.name, &detail), &mut point.enabled, status, 24.0, true);
                changed |= toggled;
                if hot && hover_button(ui, theme, r, 0, icons::X, "Remove Data Breakpoint") {
                    removed = Some(point.id.clone());
                }
            }
            if let Some(id) = removed {
                live.remove_data(&id);
            } else if changed {
                live.send_data();
            }
        }
    }
}

fn point_row(ui: &mut Ui, theme: &Theme, (name, detail): (&str, &str), enabled: &mut bool,
    status: Option<&Verification>, tail: f32, available: bool) -> (Rect, bool, bool) {
    let (r, resp) = row(ui, theme, false);
    ui::item(ui, &resp, ui::Role::ListItem, name, false);
    let rect = Rect::from_min_size(pos2(r.left() + 12.0, r.top() + 3.0), vec2(16.0, 16.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let changed = child.add_enabled_ui(available, |ui| ui::checkbox(ui, theme, enabled, "")).inner.changed();
    let mut label = r;
    label.max.x -= tail;
    let mut x = r.left() + 36.0;
    let mut tip = format!("{name} {detail}");
    if let Some(state) = status {
        let center = pos2(x + 4.0, r.center().y);
        if state.verified {
            ui.painter().circle_filled(center, 4.0, theme.error);
        } else {
            ui.painter().circle_stroke(center, 4.0, Stroke::new(1.0, theme.text_dim));
        }
        x += 14.0;
        tip.push_str(if state.verified { "\nVerified" } else { "\nUnverified" });
        if !state.message.is_empty() { tip.push_str(&format!("\n{}", state.message)); }
    }
    pair(ui, label, x, theme, name, detail, theme.text_dim);
    let hot = resp.hovered() || ui.rect_contains_pointer(r);
    resp.on_hover_text(tip);
    (r, changed, hot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Session;
    use std::time::{Duration, Instant};

    fn confirm(debug: &mut Debug, ctx: &egui::Context, marks: &mut Marks, key: Key) {
        let keys = [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
        for events in [vec![], vec![], keys.into()] {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |_| debug.confirm_breakpoint_removal(ctx, marks));
            out.textures_delta.clear();
        }
        assert!(!debug.confirm_clear);
    }

    #[test]
    fn saves_function_breakpoints_per_workspace_and_reads_old_sessions() {
        let folder = PathBuf::from("/workspace");
        let mut session: Session = serde_json::from_value(json!({"folder": folder})).unwrap();
        let mut marks = Marks::default();
        restore(&session, &mut marks);
        assert!(marks.functions.is_empty());
        let mut point = FunctionBreakpoint::new("module::run");
        point.condition = "ready".into();
        point.hit = "3".into();
        point.enabled = false;
        marks.functions.push(point.clone());
        store(&mut session, &marks);
        let mut session: Session = serde_json::from_slice(&serde_json::to_vec(&session).unwrap()).unwrap();
        session.folder = Some(PathBuf::from("/other"));
        restore(&session, &mut marks);
        assert!(marks.functions.is_empty());
        session.folder = Some(folder);
        restore(&session, &mut marks);
        assert_eq!(marks.functions, [point]);
        marks.clear();
        store(&mut session, &marks);
        assert!(session.function_breakpoints.is_empty());
    }

    #[test]
    fn builds_requests_and_interprets_adapter_replies() {
        let mut point = FunctionBreakpoint::new("main");
        point.condition = " ready ".into();
        point.hit = " 5 ".into();
        assert_eq!(point.request(&Value::Null), json!({"name": "main"}));
        assert_eq!(point.request(&json!({"supportsHitConditionalBreakpoints": true})), json!({"name": "main", "hitCondition": "5"}));
        assert_eq!(point.request(&json!({"supportsConditionalBreakpoints": true})), json!({"name": "main", "condition": "ready"}));
        let info = |body| DataBreakpoint::from_info("x".into(), &body);
        let data = info(json!({"dataId": "42", "accessTypes": ["read", "readWrite", "write"]})).unwrap();
        assert_eq!(data.request(), json!({"dataId": "42", "accessType": "write"}));
        assert_eq!(info(json!({"dataId": "x", "accessTypes": ["readWrite"]})).unwrap().request()["accessType"], "readWrite");
        assert_eq!(info(json!({"dataId": "x"})).unwrap().request(), json!({"dataId": "x"}));
        assert_eq!(info(json!({"dataId": null, "description": "Optimized out"})).unwrap_err(), "Optimized out");
        let state = verification(vec!["main".into(), "missing".into()], &json!({"breakpoints": [
            {"verified": true, "id": 0}, {"verified": false, "message": "No symbol"}
        ]}));
        assert!(state["main"].verified && state["main"].id == Some(0));
        assert!(!state["missing"].verified && state["missing"].message == "No symbol");
    }

    #[test]
    fn synchronizes_functions_and_keeps_data_with_its_adapter() {
        let file = PathBuf::from("src/main.rs");
        let adapters = ["1", "2", "4"].map(|x| super::super::tests::adapter(file.clone(), x));
        let (ctx, settings, workspace) = (egui::Context::default(), Settings::default(), Workspace::default());
        let (mut marks, mut bottom, mut tasks, mut cmds, mut panel) = (Marks::default(), BottomPanel::new(), Tasks::default(), vec![], false);
        let mut debug = Debug::default();
        macro_rules! env {
            () => { Env { folder: Some(Path::new(".")), workspace: &workspace, file: Some(file.clone()), cursor_line: None, settings: &settings,
                marks: &mut marks, bottom: &mut bottom, tasks: &mut tasks, panel: &mut panel, cmds: &mut cmds, ctx: &ctx } };
        }
        macro_rules! pump {
            ($done:expr) => {{
                let deadline = Instant::now() + Duration::from_secs(10);
                while !($done) {
                    assert!(Instant::now() < deadline, "Timed out waiting for {}", stringify!($done));
                    debug.poll(&mut env!());
                    std::thread::sleep(Duration::from_millis(5));
                }
            }};
        }
        debug.exec(&Command::AddFunctionBreakpoint, &mut env!());
        let mut frame = |events| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                debug.additional_breakpoints_ui(ui, &Theme::dark(), &mut marks);
            });
            out.textures_delta.clear();
        };
        frame(vec![]);
        frame(vec![egui::Event::Text(" main ".into())]);
        frame([true, false].map(|pressed| egui::Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() }).into());
        assert_eq!(marks.functions, [FunctionBreakpoint::new("main")]);
        marks.functions[0].hit = "4".into();
        for (i, (port, _)) in adapters.iter().enumerate() {
            let config = json!({"configurations": [{"name": format!("session-{i}"), "debugServer": port}]});
            debug.begin(parse_launch(&config.to_string()).unwrap().remove(0), false, &mut env!());
        }
        pump!(debug.live.iter().all(|l| l.state == State::Stopped && !l.scopes.is_empty()));
        assert!(debug.live[..2].iter().all(|l| l.function_status["main"].verified));
        marks.functions[0].enabled = false;
        marks.functions.push(FunctionBreakpoint::new("missing"));
        marks.functions_changed = true;
        pump!(debug.live[..2].iter().all(|l| l.function_status.contains_key("missing")));
        assert!(debug.live[..2].iter().all(|l| !l.function_status["missing"].verified && !l.function_status.contains_key("main")));
        debug.focus_on(0, &mut marks, &mut cmds);
        assert!(debug.breakpoint_commands(true).allows(&Command::BreakOnValueChange));
        debug.live[0].data_info(100, "unavailable".into());
        debug.live[0].data_info(100, "x".into());
        debug.focus_on(2, &mut marks, &mut cmds);
        assert!(!debug.breakpoint_commands(true).allows(&Command::BreakOnValueChange));
        debug.live[2].data_info(100, "x".into());
        pump!(debug.live[0].data.first().is_some_and(|b| b.status.verified));
        debug.live[0].data_info(101, "child".into());
        pump!(debug.live[0].data.len() == 2 && debug.live[0].data[1].status.verified);
        debug.live[0].data[0].enabled = false;
        debug.live[0].send_data();
        pump!(debug.live[0].data[1].status.verified);
        assert!(!debug.live[0].data[0].status.verified);
        let child = debug.live[0].data[1].id.clone();
        debug.live[0].remove_data(&child);
        assert_eq!(debug.live[0].data.len(), 1);
        pump!(!debug.live[0].wants.values().any(|w| matches!(w, Want::Data(_))));
        assert!(debug.live[1..].iter().all(|l| l.data.is_empty()));
        debug.stop(0, &mut env!());
        pump!(debug.live.len() == 2);
        assert_eq!(debug.breakpoint_count(&marks), 2);
        debug.exec(&Command::RemoveAllBreakpoints, &mut env!());
        confirm(&mut debug, &ctx, &mut marks, Key::Escape);
        assert_eq!(marks.functions.len(), 2);
        debug.exec(&Command::RemoveAllBreakpoints, &mut env!());
        confirm(&mut debug, &ctx, &mut marks, Key::Enter);
        assert!(marks.functions.is_empty());
        debug.poll(&mut env!());
        pump!(!debug.live[0].wants.values().any(|w| matches!(w, Want::Functions(_))));
        while !debug.live.is_empty() {
            let count = debug.live.len();
            debug.stop(0, &mut env!());
            pump!(debug.live.len() < count);
        }
        for (i, (_, rx)) in adapters.into_iter().enumerate() {
            let sent: Vec<_> = rx.try_iter().collect();
            let requests = |name: &str| sent.iter().filter(|r| r["command"] == name).map(|r| &r["arguments"]).collect::<Vec<_>>();
            let functions = requests("setFunctionBreakpoints");
            if i == 2 {
                assert!(functions.is_empty());
            } else {
                let first = if i == 0 { json!({"name": "main", "hitCondition": "4"}) } else { json!({"name": "main"}) };
                assert_eq!(functions[0]["breakpoints"], json!([first]));
                assert_eq!(functions[1]["breakpoints"], json!([{"name": "missing"}]));
                let at = |cmd: &str| sent.iter().position(|r| r["command"] == cmd).unwrap();
                assert!(at("setFunctionBreakpoints") < at("configurationDone"));
                if i == 1 { assert_eq!(functions[2]["breakpoints"], json!([])); }
            }
            let info = requests("dataBreakpointInfo");
            let sets = requests("setDataBreakpoints");
            if i == 0 {
                assert_eq!(info.len(), 3);
                assert_eq!(info[1], &json!({"variablesReference": 100, "name": "x"}));
                assert_eq!(info[2], &json!({"variablesReference": 101, "name": "child"}));
                let x = json!({"dataId": "1-x", "accessType": "write"});
                let child = json!({"dataId": "1-child", "accessType": "write"});
                let lists: Vec<_> = sets.iter().map(|r| &r["breakpoints"]).collect();
                assert_eq!(lists, [&json!([x]), &json!([x, child]), &json!([child]), &json!([])]);
            } else {
                assert!(info.is_empty() && sets.is_empty());
            }
        }
    }
}
