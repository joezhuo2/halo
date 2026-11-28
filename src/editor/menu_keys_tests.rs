use super::*;
use eframe::egui::{Event, PointerButton, pos2};

#[test]
fn a_context_menu_key_does_not_reach_the_focused_editor() {
    let ctx = egui::Context::default();
    let mut editors = Editors::new();
    editors.exec(&Command::NewFile, &Settings::default(), &ctx);
    tests::frame(&ctx, &mut editors, vec![]);
    tests::frame(&ctx, &mut editors, vec![Event::Paste("hello world\nsecond".into())]);
    let pos = pos2(200.0, TAB_H + 30.0);
    let click = |button, pressed| Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
    for events in [vec![Event::PointerMoved(pos)], vec![click(PointerButton::Primary, true)], vec![click(PointerButton::Primary, false)], vec![click(PointerButton::Secondary, true)], vec![click(PointerButton::Secondary, false)], vec![], vec![]] {
        tests::frame(&ctx, &mut editors, events);
    }
    let text = |e: &Editors| e.docs.values().next().unwrap().buf.rope.to_string();
    let keys = crate::ui::tests::menu_keys(&ctx).unwrap();
    assert!(keys.contains(&'g'));
    tests::frame(&ctx, &mut editors, crate::ui::tests::press('g'));
    tests::frame(&ctx, &mut editors, vec![]);
    assert_eq!(crate::ui::tests::menu_keys(&ctx), None);
    assert_eq!(text(&editors), "hello world\nsecond");
    tests::frame(&ctx, &mut editors, crate::ui::tests::press('g'));
    assert_ne!(text(&editors), "hello world\nsecond");
}
