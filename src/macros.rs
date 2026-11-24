use crate::command::Command;
use crate::ui;
use eframe::egui::{self, Event};
use std::collections::VecDeque;

type Step = (Vec<Event>, Vec<Command>);

#[derive(Default)]
pub struct Macro {
    recording: Option<Vec<Step>>,
    steps: Vec<Step>,
    playing: VecDeque<Step>,
    derived: usize,
}

impl Macro {
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn toggle(&mut self) {
        match self.recording.take() {
            Some(steps) if steps.is_empty() => ui::notify_info("Nothing was recorded, so the last macro is kept."),
            Some(steps) => {
                ui::notify_info(format!("Macro recorded: {} steps. Run Macro: Play to replay it.", steps.len()));
                self.steps = steps;
            }
            None if !self.playing.is_empty() => {}
            None => self.recording = Some(vec![]),
        }
    }

    pub fn play(&mut self) {
        match (self.recording.is_some(), self.steps.is_empty()) {
            (true, _) => ui::notify_info("Stop recording before playing the macro."),
            (_, true) => ui::notify_info("No macro is recorded yet. Run Macro: Start/Stop Recording first."),
            _ => self.playing = self.steps.iter().cloned().collect(),
        }
    }

    pub fn frame(&mut self, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        if let Some((events, commands)) = self.playing.pop_front() {
            ctx.input_mut(|i| i.events.extend(events));
            cmds.extend(commands);
            ctx.request_repaint();
        } else if let Some(steps) = &mut self.recording {
            let keys = |e: &&Event| matches!(e, Event::Key { .. } | Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut);
            steps.push((ctx.input(|i| i.events.iter().filter(keys).cloned().collect()), vec![]));
        }
    }

    pub fn derived(&mut self, queued: usize) {
        self.derived = queued;
    }

    pub fn commands(&mut self, cmds: &[Command]) {
        let skip = std::mem::take(&mut self.derived);
        let Some(steps) = &mut self.recording else { return };
        if let Some(step) = steps.last_mut() {
            step.1.extend(cmds.iter().skip(skip).filter(|c| !matches!(c, Command::ToggleMacroRecording | Command::PlayMacro)).cloned());
        }
        if steps.last().is_some_and(|s| s.0.is_empty() && s.1.is_empty()) {
            steps.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_records_keys_and_commands_then_replays_one_step_per_frame() {
        let ctx = egui::Context::default();
        let mut m = Macro::default();
        let run = |m: &mut Macro, events: Vec<Event>, cmds: Vec<Command>| {
            let mut seen = (vec![], vec![]);
            let input = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(input, |ui| {
                let mut queued = cmds.clone();
                m.frame(ui.ctx(), &mut queued);
                m.commands(&queued);
                seen = (ui.input(|i| i.events.clone()), queued);
            })
            .textures_delta
            .clear();
            seen
        };
        m.toggle();
        assert!(m.recording());
        run(&mut m, vec![Event::Text("a".into())], vec![]);
        run(&mut m, vec![], vec![]);
        run(&mut m, vec![], vec![Command::DeleteLine, Command::ToggleMacroRecording]);
        m.toggle();
        assert!(!m.recording());
        m.play();
        let (events, cmds) = run(&mut m, vec![], vec![]);
        assert!(events.contains(&Event::Text("a".into())) && cmds.is_empty());
        let (_, cmds) = run(&mut m, vec![], vec![]);
        assert_eq!(cmds, [Command::DeleteLine]);
        let (events, cmds) = run(&mut m, vec![], vec![]);
        assert!(events.is_empty() && cmds.is_empty());
    }
}
