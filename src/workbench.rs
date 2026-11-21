use crate::app::Halo;
use crate::command::Command;
use crate::terminal::PanelTab;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, CornerRadius, CursorIcon, FontId, Frame, Id, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Explorer,
    Search,
    SourceControl,
    RunAndDebug,
    Testing,
    Outline,
    Timeline,
    Ai,
    Problems,
    Output,
    DebugConsole,
    TestResults,
    Terminal,
    Ports,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Primary,
    Secondary,
    Panel,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Center,
    Justify,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Activity {
    Side,
    Top,
    Hidden,
}

use View::*;

pub const ALL: [View; 14] = [Explorer, Search, SourceControl, RunAndDebug, Testing, Outline, Timeline, Ai, Problems, Output, DebugConsole, Terminal, Ports, TestResults];
pub const MOVABLE: [View; 12] = [Explorer, SourceControl, RunAndDebug, Testing, Outline, Timeline, Ai, Problems, Output, DebugConsole, TestResults, Terminal];
pub const PLACES: [Place; 3] = [Place::Primary, Place::Secondary, Place::Panel];
pub const ACTIVITY: f32 = 46.0;
pub const STRIP: f32 = 34.0;
pub const PRIMARY: (f32, f32) = (170.0, 640.0);
pub const SECONDARY: (f32, f32) = (240.0, 800.0);
pub const PANEL: (f32, f32) = (90.0, 900.0);
pub const PANEL_WIDTH: (f32, f32) = (170.0, 900.0);
pub const RATIO: f32 = 0.62;

impl View {
    pub fn name(self) -> &'static str {
        match self {
            Explorer => "Explorer",
            Search => "Search",
            SourceControl => "Source Control",
            RunAndDebug => "Run and Debug",
            Testing => "Testing",
            Outline => "Outline",
            Timeline => "Timeline",
            Ai => "AI",
            Problems => "Problems",
            Output => "Output",
            DebugConsole => "Debug Console",
            TestResults => "Test Results",
            Terminal => "Terminal",
            Ports => "Ports",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Explorer => icons::FILES,
            Search => icons::MAGNIFYING_GLASS,
            SourceControl => icons::GIT_BRANCH,
            RunAndDebug => icons::BUG,
            Testing => icons::FLASK,
            Outline => icons::LIST_BULLETS,
            Timeline => icons::CLOCK_COUNTER_CLOCKWISE,
            Ai => icons::ROBOT,
            Problems => icons::WARNING,
            Output => icons::TEXT_ALIGN_LEFT,
            DebugConsole => icons::BUG_BEETLE,
            TestResults => icons::CHECK_SQUARE,
            Terminal => icons::TERMINAL_WINDOW,
            Ports => icons::PLUGS,
        }
    }

    pub fn home(self) -> Place {
        match self {
            Ai => Place::Secondary,
            Problems | Output | DebugConsole | TestResults | Terminal | Ports => Place::Panel,
            _ => Place::Primary,
        }
    }

    pub fn tab(self) -> Option<PanelTab> {
        Some(match self {
            Problems => PanelTab::Problems,
            Output => PanelTab::Output,
            DebugConsole => PanelTab::Debug,
            TestResults => PanelTab::Tests,
            Terminal => PanelTab::Terminal,
            Ports => PanelTab::Ports,
            _ => return None,
        })
    }

    fn tip(self) -> &'static str {
        match self {
            Explorer => "Explorer (Ctrl+Shift+E)",
            Search => "Search (Ctrl+Shift+F)",
            SourceControl => "Source Control (Ctrl+Shift+G)",
            RunAndDebug => "Run and Debug (Ctrl+Shift+D)",
            view => view.name(),
        }
    }
}

impl Place {
    pub fn name(self) -> &'static str {
        match self {
            Place::Primary => "Primary Side Bar",
            Place::Secondary => "Secondary Side Bar",
            Place::Panel => "Panel",
        }
    }
}

impl Edge {
    pub fn parse(text: &str) -> Self {
        match text {
            "left" => Edge::Left,
            "right" => Edge::Right,
            "top" => Edge::Top,
            _ => Edge::Bottom,
        }
    }

    pub fn side(text: &str) -> Self {
        if text == "right" { Edge::Right } else { Edge::Left }
    }

    pub fn flip(self) -> Self {
        match self {
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

impl Align {
    pub fn parse(text: &str) -> Self {
        match text {
            "justify" => Align::Justify,
            "left" => Align::Left,
            "right" => Align::Right,
            _ => Align::Center,
        }
    }
}

impl Activity {
    pub fn parse(text: &str) -> Self {
        match text {
            "top" | "bottom" => Activity::Top,
            "hidden" => Activity::Hidden,
            _ => Activity::Side,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Zen {
    pub shown: [bool; 3],
    pub fullscreen: bool,
    pub centered: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Workbench {
    pub moved: Vec<(View, Place)>,
    pub active: [Option<View>; 3],
    pub panel_at: Option<Edge>,
    pub panel_max: bool,
    pub primary_size: f32,
    pub secondary_size: f32,
    pub panel_size: f32,
    pub panel_width: f32,
    pub fullscreen: bool,
    pub centered: bool,
    pub centered_ratio: f32,
    pub zen: Option<Zen>,
}

impl Default for Workbench {
    fn default() -> Self {
        Self {
            moved: vec![],
            active: [None, None, Some(Terminal)],
            panel_at: None,
            panel_max: false,
            primary_size: 260.0,
            secondary_size: 380.0,
            panel_size: 240.0,
            panel_width: 420.0,
            fullscreen: false,
            centered: false,
            centered_ratio: RATIO,
            zen: None,
        }
    }
}

impl Workbench {
    pub fn place(&self, view: View) -> Place {
        self.moved.iter().find(|m| m.0 == view).map_or(view.home(), |m| m.1)
    }

    pub fn views(&self, place: Place) -> Vec<View> {
        ALL.into_iter().filter(|v| self.place(*v) == place).collect()
    }

    pub fn tops(&self, place: Place) -> Vec<View> {
        let views = self.views(place);
        let explorer = views.contains(&Explorer);
        views.into_iter().filter(|v| !(explorer && matches!(v, Outline | Timeline))).collect()
    }

    pub fn shown(&self, place: Place) -> Option<View> {
        let tops = self.tops(place);
        self.active[place as usize].filter(|v| tops.contains(v)).or(tops.first().copied())
    }

    pub fn activate(&mut self, view: View) -> Place {
        let place = self.place(view);
        let host = if self.tops(place).contains(&view) { view } else { Explorer };
        self.active[place as usize] = Some(host);
        place
    }

    pub fn move_view(&mut self, view: View, place: Place) -> bool {
        if !MOVABLE.contains(&view) || self.place(view) == place {
            return false;
        }
        self.moved.retain(|m| m.0 != view);
        if place != view.home() {
            self.moved.push((view, place));
        }
        true
    }

    pub fn reset_views(&mut self) {
        self.moved.clear();
        self.active = Self::default().active;
    }

    pub fn enter_zen(&mut self, shown: [bool; 3], full: bool, center: bool) -> [bool; 3] {
        self.zen = Some(Zen { shown, fullscreen: self.fullscreen, centered: self.centered });
        self.fullscreen |= full;
        self.centered |= center;
        [false; 3]
    }

    pub fn leave_zen(&mut self) -> Option<[bool; 3]> {
        let zen = self.zen.take()?;
        (self.fullscreen, self.centered) = (zen.fullscreen, zen.centered);
        Some(zen.shown)
    }
}

pub struct Spec {
    pub side: Edge,
    pub panel_at: Edge,
    pub align: Align,
    pub activity: Activity,
    pub primary: Option<f32>,
    pub secondary: Option<f32>,
    pub panel: Option<f32>,
    pub max: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rects {
    pub activity: Option<Rect>,
    pub primary: Option<Rect>,
    pub secondary: Option<Rect>,
    pub panel: Option<Rect>,
    pub editor: Rect,
}

fn cut(rest: &mut Rect, edge: Edge, size: f32) -> Rect {
    let mut piece = *rest;
    match edge {
        Edge::Left => {
            rest.min.x = (rest.min.x + size).min(rest.max.x);
            piece.max.x = rest.min.x;
        }
        Edge::Right => {
            rest.max.x = (rest.max.x - size).max(rest.min.x);
            piece.min.x = rest.max.x;
        }
        Edge::Top => {
            rest.min.y = (rest.min.y + size).min(rest.max.y);
            piece.max.y = rest.min.y;
        }
        Edge::Bottom => {
            rest.max.y = (rest.max.y - size).max(rest.min.y);
            piece.min.y = rest.max.y;
        }
    }
    piece
}

pub fn place(area: Rect, s: &Spec) -> Rects {
    let mut rest = area;
    let side = if s.side == Edge::Right { Edge::Right } else { Edge::Left };
    let bar = (s.activity == Activity::Side).then(|| cut(&mut rest, side, ACTIVITY));
    let align = if s.panel_at == Edge::Bottom && !s.max { s.align } else { Align::Center };
    let early = |edge: Edge| match align {
        Align::Justify => true,
        Align::Left => edge == Edge::Left,
        Align::Right => edge == Edge::Right,
        Align::Center => false,
    };
    let bars = [(side, s.primary), (side.flip(), s.secondary)];
    let mut out = [None, None];
    for (slot, (edge, size)) in bars.iter().enumerate().filter(|(_, (edge, _))| !early(*edge)) {
        out[slot] = size.map(|w| cut(&mut rest, *edge, w));
    }
    let panel = s.panel.map(|size| cut(&mut rest, s.panel_at, if s.max { f32::INFINITY } else { size }));
    for (slot, (edge, size)) in bars.iter().enumerate().filter(|(_, (edge, _))| early(*edge)) {
        out[slot] = size.map(|w| cut(&mut rest, *edge, w));
    }
    let [mut primary, secondary] = out;
    let activity = match s.activity {
        Activity::Side => bar,
        Activity::Top => primary.as_mut().map(|p| cut(p, Edge::Top, STRIP)),
        Activity::Hidden => None,
    };
    Rects { activity, primary, secondary, panel, editor: rest }
}

pub fn centered(editor: Rect, ratio: f32) -> Rect {
    Rect::from_center_size(editor.center(), vec2(editor.width() * ratio.clamp(0.2, 1.0), editor.height()))
}

fn edge_line(rect: Rect, edge: Edge) -> Rect {
    match edge {
        Edge::Left => Rect::from_x_y_ranges(rect.left() - 0.5..=rect.left() + 0.5, rect.y_range()),
        Edge::Right => Rect::from_x_y_ranges(rect.right() - 0.5..=rect.right() + 0.5, rect.y_range()),
        Edge::Top => Rect::from_x_y_ranges(rect.x_range(), rect.top() - 0.5..=rect.top() + 0.5),
        Edge::Bottom => Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 0.5..=rect.bottom() + 0.5),
    }
}

fn splitter(ui: &mut Ui, theme: &Theme, id: Id, rect: Rect, edge: Edge) -> Response {
    let line = edge_line(rect, edge);
    let resp = ui.interact(line.expand(3.0), id, Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged();
    if hot {
        ui.ctx().set_cursor_icon(if edge.vertical() { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
    }
    let t = ui::anim(ui, id.with("hot"), hot);
    ui.painter().rect_filled(line.expand(t), CornerRadius::ZERO, ui::mix(theme.border, theme.accent, t));
    resp
}

fn resize(ui: &mut Ui, theme: &Theme, id: &str, rect: Rect, edge: Edge, size: &mut f32, range: (f32, f32)) {
    let d = splitter(ui, theme, Id::new(id), rect, edge).drag_delta();
    let grow = match edge {
        Edge::Right => d.x,
        Edge::Left => -d.x,
        Edge::Bottom => d.y,
        Edge::Top => -d.y,
    };
    *size = (*size + grow).clamp(range.0, range.1);
}

fn activity_item(ui: &mut Ui, theme: &Theme, icon: &str, active: bool, tip: &str, size: egui::Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    ui::item(ui, &resp, ui::Role::Tab, tip, active);
    let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
    let a = ui::anim(ui, resp.id.with("active"), active);
    if ui.is_rect_visible(rect) {
        ui::focus_ring(ui, theme, rect, resp.id);
        let painter = ui.painter();
        if a > 0.0 {
            let bar = match size.x > size.y {
                true => Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max).shrink2(vec2(8.0 * (1.0 - a), 0.0)),
                false => Rect::from_min_max(rect.min, pos2(rect.left() + 2.0, rect.bottom())).shrink2(vec2(0.0, 8.0 * (1.0 - a))),
            };
            painter.rect_filled(bar, CornerRadius::ZERO, ui::with_alpha(theme.accent, a));
        }
        let color = ui::mix(theme.text_dim, theme.text, h.max(a));
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::proportional(size.y * 0.52), color);
    }
    resp.on_hover_text(tip)
}

fn side_menu(ui: &mut Ui, right: bool, cmds: &mut Vec<Command>) {
    ui::menu_pad(ui);
    let items = [
        (if right { "Move Primary Side Bar Left" } else { "Move Primary Side Bar Right" }, Command::ToggleSidebarPosition),
        ("Hide Primary Side Bar", Command::ToggleSidebar),
        ("Move View...", Command::MoveView),
        ("Reset View Locations", Command::ResetViewLocations),
    ];
    for (label, cmd) in items {
        if ui.button(label).clicked() {
            cmds.push(cmd);
            ui.close();
        }
    }
}

impl Halo {
    fn visible(&mut self, place: Place) -> &mut bool {
        match place {
            Place::Primary => &mut self.session.sidebar,
            Place::Secondary => &mut self.session.ai,
            Place::Panel => &mut self.session.panel,
        }
    }

    fn is_visible(&self, place: Place) -> bool {
        [self.session.sidebar, self.session.ai, self.session.panel][place as usize]
    }

    pub(crate) fn showing(&self, view: View) -> bool {
        let place = self.session.layout.place(view);
        self.is_visible(place) && self.session.layout.shown(place) == Some(view)
    }

    pub(crate) fn reveal(&mut self, view: View) {
        let place = self.session.layout.activate(view);
        *self.visible(place) = true;
    }

    pub(crate) fn toggle_view(&mut self, view: View) {
        let place = self.session.layout.place(view);
        let before = (*self.visible(place), self.session.layout.shown(place));
        self.session.layout.activate(view);
        let same = before.1 == self.session.layout.shown(place);
        *self.visible(place) = !(before.0 && same);
    }

    pub(crate) fn panel_at(&self) -> Edge {
        self.session.layout.panel_at.unwrap_or_else(|| Edge::parse(&self.settings.panel_location))
    }

    pub(crate) fn toggle_zen(&mut self) {
        let shown = match self.session.layout.leave_zen() {
            Some(shown) => shown,
            None => {
                let now = [self.session.sidebar, self.session.ai, self.session.panel];
                self.session.layout.enter_zen(now, self.settings.zen_full_screen, self.settings.zen_center_layout)
            }
        };
        (self.session.sidebar, self.session.ai, self.session.panel) = (shown[0], shown[1], shown[2]);
    }

    pub(crate) fn layout_command(&mut self, cmd: Command) {
        use Command::*;
        let layout = &mut self.session.layout;
        match cmd {
            ToggleFullScreen => layout.fullscreen ^= true,
            ToggleZenMode => self.toggle_zen(),
            ToggleCenteredLayout => layout.centered ^= true,
            ToggleSecondarySidebar => self.session.ai ^= true,
            ToggleMaximizedPanel => {
                layout.panel_max ^= true;
                self.session.panel |= self.session.layout.panel_max;
            }
            MovePanelLeft | MovePanelRight | MovePanelBottom | MovePanelTop | TogglePanelPosition => {
                let at = match cmd {
                    MovePanelLeft => Edge::Left,
                    MovePanelRight => Edge::Right,
                    MovePanelTop => Edge::Top,
                    MovePanelBottom => Edge::Bottom,
                    _ if self.panel_at().vertical() => Edge::Bottom,
                    _ => Edge::Right,
                };
                (self.session.layout.panel_at, self.session.panel) = (Some(at), true);
            }
            ToggleSidebarPosition | AlignPanelCenter | AlignPanelJustify | AlignPanelLeft | AlignPanelRight | ActivityBarSide | ActivityBarTop
            | ActivityBarHidden => {
                let s = &mut self.settings;
                match cmd {
                    ToggleSidebarPosition => s.sidebar_location = if Edge::side(&s.sidebar_location) == Edge::Left { "right" } else { "left" }.into(),
                    AlignPanelCenter => s.panel_alignment = "center".into(),
                    AlignPanelJustify => s.panel_alignment = "justify".into(),
                    AlignPanelLeft => s.panel_alignment = "left".into(),
                    AlignPanelRight => s.panel_alignment = "right".into(),
                    ActivityBarSide => s.activity_bar_location = "default".into(),
                    ActivityBarTop => s.activity_bar_location = "top".into(),
                    _ => s.activity_bar_location = "hidden".into(),
                }
                s.save();
            }
            MoveView => {
                let items = MOVABLE.map(|v| (v.name().to_owned(), self.session.layout.place(v).name().to_owned(), MoveViewTo(v)));
                self.search.pick("Select a view to move", items.into());
            }
            MoveViewTo(view) => {
                let from = self.session.layout.place(view);
                let items = PLACES.into_iter().filter(|p| *p != from).map(|p| (p.name().to_owned(), String::new(), MoveViewPlace(view, p))).collect();
                self.search.pick(&format!("Select where to move {}", view.name()), items);
            }
            MoveViewPlace(view, place) => {
                if self.session.layout.move_view(view, place) {
                    self.reveal(view);
                }
            }
            ResetViewLocations => layout.reset_views(),
            _ => {}
        }
    }

    pub(crate) fn reconcile_panel(&mut self) {
        let tab = self.bottom.tab;
        if self.seen_panel.0 != tab
            && let Some(view) = ALL.into_iter().find(|v| v.tab() == Some(tab))
        {
            if self.session.layout.place(view) != Place::Panel && !self.seen_panel.1 {
                self.session.panel = false;
            }
            self.reveal(view);
        }
        self.bottom.tab = self.session.layout.shown(Place::Panel).and_then(View::tab).unwrap_or(PanelTab::Ports);
        self.seen_panel = (self.bottom.tab, self.session.panel);
    }

    pub(crate) fn workbench(&mut self, ui: &mut Ui) {
        self.bottom.blur();
        self.reconcile_panel();
        egui::CentralPanel::default().frame(Frame::new().fill(self.theme.bg)).show(ui, |ui| {
            let layout = &self.session.layout;
            let panel_at = self.panel_at();
            let spec = Spec {
                side: Edge::side(&self.settings.sidebar_location),
                panel_at,
                align: Align::parse(&self.settings.panel_alignment),
                activity: if layout.zen.is_some() { Activity::Hidden } else { Activity::parse(&self.settings.activity_bar_location) },
                primary: self.session.sidebar.then_some(layout.primary_size),
                secondary: (self.session.ai && !layout.views(Place::Secondary).is_empty()).then_some(layout.secondary_size),
                panel: self.session.panel.then_some(if panel_at.vertical() { layout.panel_width } else { layout.panel_size }),
                max: layout.panel_max,
            };
            let r = place(ui.max_rect(), &spec);
            crate::a11y::region(ui.ctx(), Some(crate::a11y::Region::Editor));
            self.editor_area(ui, r.editor);
            crate::a11y::region(ui.ctx(), None);
            if let Some(rect) = r.activity {
                self.icon_strip(ui, rect, Place::Primary, spec.activity == Activity::Side);
            }
            let mut targets: Vec<(Place, Rect)> = r.activity.map(|a| (Place::Primary, a)).into_iter().collect();
            for (place, rect) in [(Place::Primary, r.primary), (Place::Secondary, r.secondary), (Place::Panel, r.panel)] {
                if let Some(rect) = rect {
                    let region = match place {
                        Place::Primary => Some(crate::a11y::Region::Sidebar),
                        Place::Panel => Some(crate::a11y::Region::Panel),
                        _ => None,
                    };
                    crate::a11y::region(ui.ctx(), region);
                    self.container(ui, place, rect);
                    crate::a11y::region(ui.ctx(), None);
                    targets.push((place, rect));
                }
            }
            let theme = self.theme.clone();
            let layout = &mut self.session.layout;
            if let Some(rect) = r.primary {
                let rect = r.activity.filter(|_| spec.activity == Activity::Top).map_or(rect, |a| a.union(rect));
                resize(ui, &theme, "primary_split", rect, spec.side.flip(), &mut layout.primary_size, PRIMARY);
            }
            if let Some(rect) = r.secondary {
                resize(ui, &theme, "secondary_split", rect, spec.side, &mut layout.secondary_size, SECONDARY);
            }
            if let Some(rect) = r.panel.filter(|_| !spec.max) {
                let (size, range) = if panel_at.vertical() { (&mut layout.panel_width, PANEL_WIDTH) } else { (&mut layout.panel_size, PANEL) };
                resize(ui, &theme, "panel_split", rect, panel_at.flip(), size, range);
            }
            self.drop_view(ui, &targets);
        });
        if !self.showing(Ai) {
            self.ai.blur();
        }
    }

    fn editor_area(&mut self, ui: &mut Ui, area: Rect) {
        if area.width() < 1.0 || area.height() < 1.0 {
            return;
        }
        let layout = &self.session.layout;
        let zen = layout.zen.is_some();
        let center = layout.centered && self.editors.columns() == 1;
        let rect = if center { centered(area, layout.centered_ratio) } else { area };
        let frame = if zen && self.settings.zen_show_tabs == "none" && !self.editors.is_empty() { rect.with_min_y(rect.top() - 34.0) } else { rect };
        let plain = (zen && self.settings.zen_hide_line_numbers).then(|| {
            let mut s = self.settings.clone();
            s.line_numbers = "off".into();
            s.langs.values_mut().for_each(|l| l.line_numbers = "off".into());
            s
        });
        let ai = self.showing(Ai);
        ui.scope_builder(UiBuilder::new().max_rect(frame).id_salt("editor_area"), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            self.editors.poll_background(ui.ctx());
            if self.editors.is_empty() {
                self.welcome(ui);
            } else {
                self.editors.ai_open = ai;
                self.editors.ui(ui, &self.theme, plain.as_ref().unwrap_or(&self.settings), &self.git, &mut self.cmds);
            }
            self.drop_files(ui, rect);
        });
        if center {
            let theme = self.theme.clone();
            for (edge, sign) in [(Edge::Left, -1.0), (Edge::Right, 1.0)] {
                let resp = splitter(ui, &theme, Id::new(("centered_margin", sign as i32)), rect, edge);
                let ratio = &mut self.session.layout.centered_ratio;
                *ratio = (*ratio + sign * 2.0 * resp.drag_delta().x / area.width()).clamp(0.2, 1.0);
                if resp.double_clicked() {
                    *ratio = RATIO;
                }
            }
        }
    }

    fn container(&mut self, ui: &mut Ui, place: Place, rect: Rect) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, self.theme.panel);
        let tops = self.session.layout.tops(place);
        let Some(mut view) = self.session.layout.shown(place) else { return };
        let mut body = rect;
        if place == Place::Panel {
            let names: Vec<String> = tops.iter().map(|v| v.name().to_uppercase()).collect();
            let labels: Vec<&str> = names.iter().map(String::as_str).collect();
            let mut index = tops.iter().position(|v| *v == view).unwrap_or(0);
            let cwd = self.session.folder.clone();
            let (rest, dragged) = ui
                .scope_builder(UiBuilder::new().max_rect(rect).id_salt("panel_header"), |ui| {
                    self.bottom.header(ui, &self.theme, &self.settings, cwd.as_deref(), &labels, &mut index, view.tab(), &mut self.cmds)
                })
                .inner;
            (body, view) = (rest, tops[index]);
            self.session.layout.active[Place::Panel as usize] = Some(view);
            if let Some(dragged) = dragged.map(|i| tops[i]).filter(|v| MOVABLE.contains(v)) {
                self.dragging = Some(dragged);
            }
        } else if place == Place::Secondary && tops.len() > 1 {
            let strip = Rect::from_min_size(rect.min, vec2(rect.width(), STRIP));
            body.min.y = strip.bottom();
            self.icon_strip(ui, strip, place, false);
        }
        self.view_ui(ui, view, body);
    }

    fn icon_strip(&mut self, ui: &mut Ui, rect: Rect, place: Place, vertical: bool) {
        let theme = self.theme.clone();
        let size = if vertical { vec2(ACTIVITY, 44.0) } else { vec2(36.0, STRIP) };
        let tops = self.session.layout.tops(place);
        let shown = self.session.layout.shown(place).filter(|_| self.is_visible(place));
        let primary = place == Place::Primary;
        let right = Edge::side(&self.settings.sidebar_location) == Edge::Right;
        ui.painter().rect_filled(rect, CornerRadius::ZERO, if vertical { theme.bar } else { theme.panel });
        if primary {
            ui.interact(rect, Id::new("activity_bar"), Sense::click()).context_menu(|ui| side_menu(ui, right, &mut self.cmds));
        }
        let mut clicked = None;
        let layout = if vertical { Layout::top_down(egui::Align::Center) } else { Layout::left_to_right(egui::Align::Center) };
        ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout).id_salt(("icon_strip", place as u8)), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for view in &tops {
                let resp = activity_item(ui, &theme, view.icon(), shown == Some(*view), view.tip(), size);
                if resp.drag_started() && MOVABLE.contains(view) {
                    self.dragging = Some(*view);
                }
                if resp.clicked() {
                    clicked = Some(*view);
                }
                if primary {
                    resp.context_menu(|ui| side_menu(ui, right, &mut self.cmds));
                }
            }
            if !primary {
                return;
            }
            ui.with_layout(if vertical { Layout::bottom_up(egui::Align::Center) } else { Layout::right_to_left(egui::Align::Center) }, |ui| {
                if activity_item(ui, &theme, icons::GEAR, false, "Settings (Ctrl+,)", size).clicked() {
                    self.cmds.push(Command::OpenSettings);
                }
                let accounts = activity_item(ui, &theme, icons::USER_CIRCLE, false, "Accounts", size);
                egui::Popup::menu(&accounts).show(|ui| self.accounts.menu(ui));
                if cfg!(windows) {
                    let folder = self.session.folder.clone();
                    let in_wsl = folder.as_deref().and_then(crate::wsl::parse).is_some();
                    let button = activity_item(ui, &theme, icons::LINUX_LOGO, in_wsl, "Switch to WSL", size);
                    egui::Popup::menu(&button).show(|ui| self.wsl.menu(ui, folder.as_deref(), &mut self.cmds));
                }
            });
        });
        match clicked {
            Some(Search) => self.cmds.push(Command::FindInFiles),
            Some(view) if primary => self.toggle_view(view),
            Some(view) => self.reveal(view),
            None => {}
        }
    }

    fn view_ui(&mut self, ui: &mut Ui, view: View, rect: Rect) {
        let cwd = self.session.folder.clone();
        let active = self.editors.active_path();
        ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(("view", view as u8)), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            match view {
                Explorer => self.explorer_view(ui, active.as_deref()),
                Search => self.find.ui(ui, &self.theme, &self.session.workspace.prefixed(), &mut self.editors, &mut self.cmds),
                SourceControl => self.scm.ui(ui, &self.theme, &mut self.git, &mut self.cmds),
                RunAndDebug => self.debug_view(ui),
                Testing => self.testing_view(ui),
                Outline => self.outline_view(ui),
                Timeline => self.timeline.ui(ui, &self.theme, active.as_deref(), &mut self.session.timeline, &mut self.cmds),
                Ai => self.ai.ui(ui, &self.theme, self.settings.font_size, cwd.as_deref(), &mut self.cmds),
                _ => {
                    let files = self.tasks.files();
                    let tab = view.tab().unwrap_or(PanelTab::Ports);
                    self.bottom.body(ui, rect, tab, &self.theme, &self.settings, cwd.as_deref(), &self.editors.lsp, &files, &mut self.cmds);
                }
            }
        });
    }

    fn outline_view(&mut self, ui: &mut Ui) {
        let (active, loading) = (!self.editors.is_empty(), self.editors.outline_loading());
        crate::app::outline(ui, &self.theme, self.editors.outline(), active, loading, &mut self.session.outline, &mut self.cmds);
    }

    fn explorer_view(&mut self, ui: &mut Ui, active: Option<&std::path::Path>) {
        let views = self.session.layout.views(self.session.layout.place(Explorer));
        let (has_outline, has_timeline) = (views.contains(&Outline), views.contains(&Timeline));
        let full = ui.available_rect_before_wrap();
        let timeline_t = if has_timeline { ui::reveal(ui, Id::new("timeline_reveal"), self.session.timeline) } else { 0.0 };
        let outline_t = ui::reveal(ui, Id::new("outline_reveal"), self.session.outline);
        let size = |t: f32, share: f32| 24.0 + t * (full.height() * share - 24.0).max(0.0);
        let lower = match has_timeline {
            true => (full.bottom() - size(timeline_t, 0.3)).max(full.top() + 104.0).min(full.bottom() - 24.0),
            false => full.bottom(),
        };
        let split = match has_outline {
            true => (lower - size(outline_t, 0.4 - 0.1 * timeline_t)).max(full.top() + 80.0).min(lower - 24.0),
            false => lower,
        };
        ui.scope_builder(UiBuilder::new().max_rect(full.with_max_y(split)), |ui| {
            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
            let tabs = self.editors.open_tabs();
            self.explorer.ui(ui, &self.theme, &self.git, active, &self.settings, &tabs, &mut self.cmds);
        });
        let sections = [(Outline, has_outline, full.with_min_y(split).with_max_y(lower)), (Timeline, has_timeline, full.with_min_y(lower))];
        for (view, _, region) in sections.into_iter().filter(|s| s.1) {
            // registered under the header so its click still toggles; only a real drag moves the view
            let header = Rect::from_min_size(region.min, vec2(region.width(), 24.0));
            let handle = ui.interact(header, Id::new(("view_drag", view as u8)), Sense::drag());
            if handle.dragged() && self.dragging.is_none() && ui.input(|i| i.pointer.is_decidedly_dragging()) {
                self.dragging = Some(view);
            }
            ui.scope_builder(UiBuilder::new().max_rect(region), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                match view {
                    Outline => self.outline_view(ui),
                    _ => self.timeline.ui(ui, &self.theme, active, &mut self.session.timeline, &mut self.cmds),
                }
            });
        }
    }

    fn drop_view(&mut self, ui: &Ui, targets: &[(Place, Rect)]) {
        let Some(view) = self.dragging else { return };
        let ctx = ui.ctx().clone();
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        let pos = ctx.pointer_latest_pos();
        let target = pos.and_then(|p| targets.iter().find(|(_, r)| r.contains(p))).copied();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("view_drop")));
        let theme = &self.theme;
        if let Some((_, rect)) = target {
            painter.rect(rect, CornerRadius::ZERO, ui::with_alpha(theme.accent, 0.12), Stroke::new(1.0, theme.accent), StrokeKind::Inside);
        }
        if let Some(p) = pos {
            let galley = painter.layout_no_wrap(view.name().to_owned(), FontId::proportional(12.5), theme.text);
            let label = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(14.0, 8.0));
            painter.rect(label, CornerRadius::same(4), theme.raised, Stroke::new(1.0, theme.border), StrokeKind::Inside);
            painter.galley(label.min + vec2(7.0, 4.0), galley, theme.text);
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.dragging = None;
            if let Some((place, _)) = target
                && self.session.layout.move_view(view, place)
            {
                self.reveal(view);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn area(r: Rect) -> f32 {
        r.width().max(0.0) * r.height().max(0.0)
    }

    #[test]
    fn placement_tiles_the_window_for_every_combination() {
        let window = Rect::from_min_max(pos2(0.0, 36.0), pos2(1600.0, 976.0));
        for side in [Edge::Left, Edge::Right] {
            for panel_at in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
                for align in [Align::Center, Align::Justify, Align::Left, Align::Right] {
                    for activity in [Activity::Side, Activity::Top, Activity::Hidden] {
                        for (primary, secondary, panel, max) in [
                            (true, true, true, false),
                            (true, false, true, false),
                            (false, true, true, false),
                            (true, true, false, false),
                            (false, false, false, false),
                            (true, true, true, true),
                        ] {
                            let s = Spec {
                                side,
                                panel_at,
                                align,
                                activity,
                                primary: primary.then_some(260.0),
                                secondary: secondary.then_some(380.0),
                                panel: panel.then_some(if panel_at.vertical() { 300.0 } else { 240.0 }),
                                max,
                            };
                            let r = place(window, &s);
                            let case = format!("{side:?} {panel_at:?} {align:?} {activity:?} {primary} {secondary} {panel} {max}");
                            let parts: Vec<Rect> = [r.activity, r.primary, r.secondary, r.panel, Some(r.editor)].into_iter().flatten().collect();
                            assert!((parts.iter().map(|p| area(*p)).sum::<f32>() - area(window)).abs() < 0.5, "{case}");
                            for (i, a) in parts.iter().enumerate() {
                                assert!(window.contains_rect(*a), "{case}");
                                for b in &parts[i + 1..] {
                                    assert!(area(a.intersect(*b)) < 0.01, "{case}");
                                }
                            }
                            assert_eq!(r.primary.is_some(), primary, "{case}");
                            assert_eq!(r.activity.is_some(), activity == Activity::Side || (activity == Activity::Top && primary), "{case}");
                            if let Some(p) = r.primary {
                                assert!(if side == Edge::Left { p.center().x < r.editor.center().x } else { p.center().x > r.editor.center().x }, "{case}");
                                assert_eq!(p.width(), 260.0, "{case}");
                            }
                            if let (Some(s), Some(p)) = (r.secondary, r.primary) {
                                assert!((s.center().x - r.editor.center().x).signum() != (p.center().x - r.editor.center().x).signum(), "{case}");
                            }
                            if let Some(a) = r.activity.filter(|_| activity == Activity::Side) {
                                assert_eq!(a.height(), window.height(), "{case}");
                                assert_eq!(if side == Edge::Left { a.left() } else { a.right() }, if side == Edge::Left { window.left() } else { window.right() }, "{case}");
                            }
                            if let (Some(a), Some(p)) = (r.activity.filter(|_| activity == Activity::Top), r.primary) {
                                assert!(a.bottom() == p.top() && a.width() == p.width() && a.height() == STRIP, "{case}");
                            }
                            match (r.panel, max) {
                                (Some(_), true) => assert_eq!(area(r.editor), 0.0, "{case}"),
                                (Some(p), false) => {
                                    let touches = match panel_at {
                                        Edge::Bottom => p.top() == r.editor.bottom(),
                                        Edge::Top => p.bottom() == r.editor.top(),
                                        Edge::Left => p.right() == r.editor.left(),
                                        Edge::Right => p.left() == r.editor.right(),
                                    };
                                    assert!(touches, "{case}");
                                    if panel_at == Edge::Bottom {
                                        assert_eq!(p.bottom(), window.bottom(), "{case}");
                                    }
                                }
                                (None, _) => assert!(area(r.editor) > 0.0, "{case}"),
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn panel_alignment_decides_what_the_bottom_panel_runs_under() {
        let window = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 800.0));
        let spec = |align, side| Spec { side, panel_at: Edge::Bottom, align, activity: Activity::Side, primary: Some(200.0), secondary: Some(300.0), panel: Some(250.0), max: false };
        let panel = |align, side| place(window, &spec(align, side)).panel.unwrap();
        assert_eq!(panel(Align::Center, Edge::Left).x_range(), (246.0..=700.0));
        assert_eq!(panel(Align::Justify, Edge::Left).x_range(), (46.0..=1000.0));
        assert_eq!(panel(Align::Left, Edge::Left).x_range(), (46.0..=700.0));
        assert_eq!(panel(Align::Right, Edge::Left).x_range(), (246.0..=1000.0));
        assert_eq!(panel(Align::Left, Edge::Right).x_range(), (0.0..=754.0));
        let r = place(window, &spec(Align::Justify, Edge::Right));
        assert_eq!((r.activity.unwrap().left(), r.primary.unwrap().left(), r.secondary.unwrap().left()), (954.0, 754.0, 0.0));
        assert_eq!(r.primary.unwrap().bottom(), 550.0);
        let top = place(window, &Spec { panel_at: Edge::Top, align: Align::Justify, ..spec(Align::Justify, Edge::Left) });
        assert_eq!(top.panel.unwrap().x_range(), 246.0..=700.0);
        assert_eq!(top.panel.unwrap().top(), 0.0);
        let max = place(window, &Spec { max: true, ..spec(Align::Justify, Edge::Left) });
        assert_eq!(max.panel.unwrap(), Rect::from_min_max(pos2(246.0, 0.0), pos2(700.0, 800.0)));
    }

    #[test]
    fn centered_editor_keeps_its_middle() {
        let editor = Rect::from_min_max(pos2(100.0, 0.0), pos2(1100.0, 500.0));
        let c = centered(editor, 0.5);
        assert_eq!((c.left(), c.right(), c.height()), (350.0, 850.0, 500.0));
        assert_eq!(centered(editor, 3.0), editor);
        assert_eq!(centered(editor, 0.0).width(), 200.0);
    }

    #[test]
    fn zen_restores_what_was_visible() {
        let mut w = Workbench { centered: false, fullscreen: true, ..Workbench::default() };
        assert_eq!(w.enter_zen([true, false, true], true, true), [false; 3]);
        assert!(w.fullscreen && w.centered && w.zen.is_some());
        assert_eq!(w.leave_zen(), Some([true, false, true]));
        assert!(w.fullscreen && !w.centered && w.zen.is_none());
        assert_eq!(w.leave_zen(), None);
        let mut w = Workbench::default();
        w.enter_zen([false, true, false], false, false);
        assert!(!w.fullscreen && !w.centered);
        let back: Workbench = serde_json::from_str(&serde_json::to_string(&w).unwrap()).unwrap();
        assert_eq!(back.zen, Some(Zen { shown: [false, true, false], fullscreen: false, centered: false }));
    }

    fn run(ctx: &egui::Context, app: &mut Halo, time: f64, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))), time: Some(time), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
    }

    fn headless() -> (egui::Context, Halo) {
        let ctx = egui::Context::default();
        let mut app = Halo::build(&ctx, crate::settings::Settings::default(), crate::settings::Session::default());
        app.session.layout.active[Place::Panel as usize] = Some(Problems);
        (ctx, app)
    }

    #[test]
    fn headless_moves_a_view_to_the_secondary_side_bar_and_back() {
        let (ctx, mut app) = headless();
        run(&ctx, &mut app, 0.0, vec![]);
        assert!(app.session.sidebar && !app.session.ai && app.showing(Explorer));
        app.cmds.push(Command::MoveViewPlace(Explorer, Place::Secondary));
        run(&ctx, &mut app, 0.1, vec![]);
        run(&ctx, &mut app, 0.2, vec![]);
        assert_eq!(app.session.layout.place(Explorer), Place::Secondary);
        assert!(app.session.ai && app.showing(Explorer) && !app.showing(Ai));
        assert_eq!(app.session.layout.tops(Place::Primary)[..2], [Search, SourceControl]);
        app.cmds.push(Command::ToggleAi);
        run(&ctx, &mut app, 0.3, vec![]);
        assert!(app.session.ai && app.showing(Ai));
        app.cmds.push(Command::MoveViewPlace(Explorer, Place::Primary));
        run(&ctx, &mut app, 0.4, vec![]);
        run(&ctx, &mut app, 0.5, vec![]);
        assert!(app.session.layout.moved.is_empty() && app.showing(Explorer) && app.showing(Ai));
        app.cmds.extend([Command::MoveViewPlace(TestResults, Place::Secondary), Command::ToggleAi]);
        run(&ctx, &mut app, 0.6, vec![]);
        assert!(app.showing(Ai) && !app.showing(TestResults));
        app.session.panel = false;
        run(&ctx, &mut app, 0.7, vec![]);
        app.bottom.show_tests();
        app.session.panel = true;
        run(&ctx, &mut app, 0.8, vec![]);
        assert!(app.showing(TestResults) && !app.session.panel);
        app.cmds.push(Command::ResetViewLocations);
        run(&ctx, &mut app, 0.9, vec![]);
        assert_eq!(app.session.layout.place(TestResults), Place::Panel);
    }

    fn open_changes_in(dir: std::path::PathBuf) {
        let run_git = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        run_git(&["init", "-q", "-b", "trunk"]);
        run_git(&["config", "user.email", "t@example.com"]);
        run_git(&["config", "user.name", "t"]);
        let file = dir.join("src").join("a.cpp");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        run_git(&["add", "."]);
        run_git(&["commit", "-q", "-m", "init"]);
        std::fs::write(&file, "one\nTWO\n").unwrap();

        let (ctx, mut app) = headless();
        app.cmds.push(Command::OpenPath(dir.clone()));
        run(&ctx, &mut app, 0.0, vec![]);
        app.git.wait(|g| g.changes(false).len() == 1);
        let diff = Command::OpenDiff("a.cpp (changes)".into(), ["diff", "--", "src/a.cpp"].map(str::to_owned).to_vec());
        let mut time = 0.1;
        let mut settle = |app: &mut Halo, what: &str, done: &dyn Fn(&Halo) -> bool| {
            for _ in 0..300 {
                time += 0.05;
                run(&ctx, app, time, vec![]);
                if done(app) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("timed out waiting for {what}: open {:?}, active {:?}, hunks {:?}, top {:?}", app.editors.open_paths(), app.editors.active_path(), app.editors.active_diff_hunks(), app.git.top());
        };
        let diff_shown = |app: &Halo| app.editors.active_diff_hunks() == Some(1);
        app.cmds.push(diff.clone());
        settle(&mut app, "the diff of a closed file", &diff_shown);
        app.cmds.push(Command::CloseAllEditors);
        settle(&mut app, "no editors", &|app| app.editors.is_empty());
        app.cmds.push(Command::OpenPath(file.clone()));
        settle(&mut app, "the file", &|app| app.editors.active_path().is_some());
        app.cmds.push(diff);
        settle(&mut app, "the diff of an open file", &diff_shown);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_opens_changes_from_source_control_whether_the_file_is_open_or_not() {
        open_changes_in(std::env::temp_dir().join(format!("halo-open-changes-{}", std::process::id())));
    }

    #[test]
    #[ignore = "needs git inside the Ubuntu-24.04 WSL distro"]
    fn headless_opens_changes_in_wsl() {
        open_changes_in(std::path::PathBuf::from(format!(r"\\wsl.localhost\Ubuntu-24.04\var\tmp\halo-open-changes-{}", std::process::id())));
    }

    #[test]
    fn headless_clicking_a_section_header_toggles_it_without_dragging_the_view() {
        let (ctx, mut app) = headless();
        run(&ctx, &mut app, 0.0, vec![]);
        run(&ctx, &mut app, 0.1, vec![]);
        let open = app.session.outline;
        let at = ctx.read_response(Id::new(("view_drag", Outline as u8))).unwrap().rect.center();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&ctx, &mut app, 0.2, vec![egui::Event::PointerMoved(at), button(true)]);
        assert!(app.dragging.is_none());
        run(&ctx, &mut app, 0.25, vec![button(false)]);
        run(&ctx, &mut app, 0.3, vec![]);
        assert_eq!(app.session.outline, !open);
        assert!(app.dragging.is_none() && app.session.layout.moved.is_empty());
        run(&ctx, &mut app, 1.0, vec![button(true)]);
        run(&ctx, &mut app, 1.1, vec![egui::Event::PointerMoved(at + vec2(0.0, -60.0))]);
        assert_eq!(app.dragging, Some(Outline));
    }

    #[test]
    fn headless_zen_mode_hides_everything_and_escape_twice_restores_it() {
        let (ctx, mut app) = headless();
        app.session.ai = true;
        run(&ctx, &mut app, 0.0, vec![]);
        app.cmds.push(Command::ToggleZenMode);
        run(&ctx, &mut app, 0.1, vec![]);
        run(&ctx, &mut app, 0.2, vec![]);
        assert!(!app.session.sidebar && !app.session.ai && !app.session.panel);
        assert!(app.session.layout.zen.is_some() && app.session.layout.fullscreen && app.session.layout.centered);
        app.cmds.push(Command::ToggleSidebar);
        run(&ctx, &mut app, 0.3, vec![]);
        assert!(app.session.sidebar);
        let escape = || vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }];
        run(&ctx, &mut app, 1.0, escape());
        assert!(app.session.layout.zen.is_some());
        run(&ctx, &mut app, 1.2, escape());
        assert!(app.session.layout.zen.is_none() && !app.session.layout.fullscreen && !app.session.layout.centered);
        assert!(app.session.sidebar && app.session.ai && app.session.panel);
    }

    #[test]
    fn views_move_reset_and_round_trip() {
        let mut w = Workbench::default();
        assert_eq!(w.views(Place::Secondary), [Ai]);
        assert_eq!(w.tops(Place::Primary), [Explorer, Search, SourceControl, RunAndDebug, Testing]);
        assert_eq!(w.shown(Place::Panel), Some(Terminal));
        assert_eq!(w.tops(Place::Panel), [Problems, Output, DebugConsole, Terminal, Ports, TestResults]);
        assert!(!w.move_view(Search, Place::Panel) && !w.move_view(Ports, Place::Primary) && !w.move_view(Ai, Place::Secondary));
        assert!(w.move_view(Outline, Place::Secondary) && w.move_view(Terminal, Place::Primary));
        assert_eq!(w.views(Place::Secondary), [Outline, Ai]);
        assert_eq!(w.activate(Terminal), Place::Primary);
        assert_eq!(w.shown(Place::Primary), Some(Terminal));
        assert!(w.move_view(Explorer, Place::Panel));
        assert_eq!(w.tops(Place::Primary), [Search, SourceControl, RunAndDebug, Testing, Timeline, Terminal]);
        assert_eq!(w.activate(Timeline), Place::Primary);
        assert_eq!(w.shown(Place::Primary), Some(Timeline));
        assert!(w.move_view(Explorer, Place::Primary));
        assert_eq!(w.shown(Place::Primary), Some(Explorer));
        assert_eq!(w.activate(Timeline), Place::Primary);
        assert_eq!(w.active[0], Some(Explorer));
        assert!(w.move_view(Terminal, Place::Panel));
        assert_eq!(w.moved, [(Outline, Place::Secondary)]);
        assert!(w.move_view(Ai, Place::Panel) && w.move_view(Outline, Place::Primary));
        assert!(w.views(Place::Secondary).is_empty() && w.shown(Place::Secondary).is_none());
        let back: Workbench = serde_json::from_str(&serde_json::to_string(&w).unwrap()).unwrap();
        assert_eq!(back, w);
        w.reset_views();
        assert!(w.moved.is_empty() && w.active == Workbench::default().active && w.place(Ai) == Place::Secondary);
        let old: crate::settings::Session = serde_json::from_str(r#"{"sidebar":true,"search":true,"debug_view":false,"ai":true}"#).unwrap();
        assert!(old.ai && old.layout == Workbench::default());
        let partial: Workbench = serde_json::from_str(r#"{"moved":[["Problems","Secondary"]],"panel_at":"Left"}"#).unwrap();
        assert_eq!((partial.place(Problems), partial.panel_at, partial.primary_size), (Place::Secondary, Some(Edge::Left), 260.0));
    }
}
