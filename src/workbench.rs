use crate::ui::icons;
use eframe::egui::{Rect, vec2};
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

pub const ALL: [View; 14] = [Explorer, Search, SourceControl, RunAndDebug, Testing, Outline, Timeline, Ai, Problems, Output, DebugConsole, TestResults, Terminal, Ports];
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
            active: [None; 3],
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
        self.active = [None; 3];
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

    #[test]
    fn views_move_reset_and_round_trip() {
        let mut w = Workbench::default();
        assert_eq!(w.views(Place::Secondary), [Ai]);
        assert_eq!(w.tops(Place::Primary), [Explorer, Search, SourceControl, RunAndDebug, Testing]);
        assert_eq!(w.shown(Place::Panel), Some(Problems));
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
        assert!(w.moved.is_empty() && w.active == [None; 3] && w.place(Ai) == Place::Secondary);
        let old: crate::settings::Session = serde_json::from_str(r#"{"sidebar":true,"search":true,"debug_view":false,"ai":true}"#).unwrap();
        assert!(old.ai && old.layout == Workbench::default());
        let partial: Workbench = serde_json::from_str(r#"{"moved":[["Problems","Secondary"]],"panel_at":"Left"}"#).unwrap();
        assert_eq!((partial.place(Problems), partial.panel_at, partial.primary_size), (Place::Secondary, Some(Edge::Left), 260.0));
    }
}
