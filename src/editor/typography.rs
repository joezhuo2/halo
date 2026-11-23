use eframe::egui::{Context, Galley, Vec2};
use eframe::egui::epaint::text::{ByteIndex, LayoutJob};

#[derive(Clone, Copy)]
pub(super) struct FontStyle {
    ligatures: bool,
    pub(super) spacing: f32,
}

impl FontStyle {
    pub(super) fn new(settings: &crate::settings::Settings, glyph_width: f32) -> Self {
        Self { ligatures: settings.font_ligatures, spacing: settings.letter_spacing.max(1.0 - glyph_width) }
    }

    pub(super) fn apply(self, job: LayoutJob) -> LayoutJob {
        layout(job, self.ligatures, self.spacing)
    }
}

fn layout(mut job: LayoutJob, ligatures: bool, spacing: f32) -> LayoutJob {
    if ligatures {
        job.sections.dedup_by(|next, prev| {
            if next.leading_space == 0.0 && prev.byte_range.end == next.byte_range.start && prev.format == next.format {
                prev.byte_range.end = next.byte_range.end;
                true
            } else {
                false
            }
        });
        if spacing == 0.0 { return job; }
    }
    let mut after_tab = false;
    for mut section in std::mem::take(&mut job.sections) {
        section.format.extra_letter_spacing = spacing;
        let range = section.byte_range.start.0..section.byte_range.end.0;
        let mut start = range.start;
        for (i, c) in job.text[range.clone()].char_indices() {
            let byte = range.start + i;
            if start < byte && (!ligatures || (after_tab && spacing != 0.0)) {
                section.byte_range = ByteIndex(start)..ByteIndex(byte);
                job.sections.push(section.clone());
                start = byte;
                section.leading_space = 0.0;
            }
            if after_tab {
                section.leading_space += 3.0 * spacing;
            }
            after_tab = c == '\t';
        }
        section.byte_range = ByteIndex(start)..ByteIndex(range.end);
        job.sections.push(section);
    }
    job
}

pub(super) fn x(galley: &Galley, col: usize) -> f32 {
    let Some(row) = galley.rows.first() else { return 0.0 };
    let glyphs = &row.glyphs;
    let Some(glyph) = glyphs.get(col) else {
        let Some(last) = glyphs.last() else { return 0.0 };
        let spacing = galley.job.sections.last().map_or(0.0, |s| s.format.extra_letter_spacing);
        return row.size.x + if last.chr == '\t' { 4.0 * spacing } else { spacing };
    };
    let continuation = |i: usize| glyphs[i].advance_width == 0.0 && glyphs[i].chr.is_ascii_graphic();
    if !continuation(col) || col == 0 { return glyph.pos.x; }
    let start = (0..col).rev().find(|i| !continuation(*i)).unwrap_or(0);
    let end = (col + 1..glyphs.len()).find(|i| !continuation(*i)).unwrap_or(glyphs.len());
    let base = &glyphs[start];
    base.pos.x + base.advance_width * (col - start) as f32 / (end - start) as f32
}

pub(super) fn hit(galley: &Galley, target: f32) -> usize {
    let mut low = 0;
    let mut high = galley.rows.first().map_or(0, |row| row.glyphs.len());
    while low < high {
        let mid = low + (high - low) / 2;
        if target < (x(galley, mid) + x(galley, mid + 1)) * 0.5 {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

#[derive(Clone, Copy)]
pub(super) struct Caret {
    head: usize,
    version: u64,
    from: Vec2,
    to: Vec2,
    start: f64,
}

impl Caret {
    fn position(&self, now: f64) -> Vec2 {
        let t = ((now - self.start) / 0.080).clamp(0.0, 1.0) as f32;
        self.to + (self.from - self.to) * (1.0 - t).powi(2)
    }

    pub(super) fn position_for(slot: &mut Option<Self>, ctx: &Context, to: Vec2, head: usize, version: u64, mode: &str, now: f64) -> Vec2 {
        let caret = slot.get_or_insert(Self { head, version, from: to, to, start: now });
        let animate = !crate::a11y::reduced() && (mode == "on" || (mode == "explicit" && caret.version == version));
        if !animate || caret.to != to {
            caret.from = if animate && (caret.head != head || caret.version != version) { caret.position(now) } else { to };
            caret.to = to;
            caret.start = now;
        }
        caret.head = head;
        caret.version = version;
        let pos = caret.position(now);
        if pos != to { ctx.request_repaint(); }
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Color32, FontId, vec2};

    #[test]
    fn caret_glides_and_finishes_in_eighty_milliseconds() {
        let caret = Caret { head: 0, version: 1, from: Vec2::ZERO, to: vec2(80.0, 40.0), start: 0.0 };
        assert_eq!(caret.position(0.0), Vec2::ZERO);
        assert_eq!(caret.position(0.04), vec2(60.0, 30.0));
        assert_eq!(caret.position(0.08), caret.to);
        assert_eq!(caret.position(1.0), caret.to);
    }

    #[test]
    fn installed_cascadia_shapes_operators_and_keeps_each_caret_stop() {
        let Some((_, file)) = crate::settings::font_files().into_iter().find(|(name, _)| *name == "Cascadia Code") else { return };
        let ctx = Context::default();
        ctx.set_fonts(crate::theme::fonts(&file, ""));
        ctx.run_ui(Default::default(), |ui| {
            for spacing in [0.0, 2.0] {
                let make = |on| {
                    let job = LayoutJob::simple_singleline("a=>b".into(), FontId::monospace(20.0), Color32::WHITE);
                    ui.painter().layout_job(layout(job, on, spacing))
                };
                let (on, off) = (make(true), make(false));
                let uvs = |g: &Galley| g.rows[0].glyphs.iter().map(|glyph| glyph.uv_rect.min).collect::<Vec<_>>();
                assert_ne!(uvs(&on), uvs(&off));
                for col in 0..=4 {
                    assert_eq!(hit(&on, x(&on, col)), col);
                    if col > 0 { assert!(x(&on, col) > x(&on, col - 1)); }
                }
            }
        }).textures_delta.clear();
    }

    #[test]
    fn spacing_moves_carets_across_shaping_sections() {
        let ctx = Context::default();
        ctx.run_ui(Default::default(), |ui| {
            for ligatures in [false, true] {
                let make = |spacing| {
                    let job = LayoutJob::simple_singleline("==>x".into(), FontId::monospace(16.0), Color32::WHITE);
                    let job = layout(job, ligatures, spacing);
                    assert_eq!(job.sections.len(), if ligatures { 1 } else { 4 });
                    ui.painter().layout_job(job)
                };
                let (tight, loose) = (make(0.0), make(3.0));
                for col in 1..=4 {
                    assert!((x(&loose, col) - x(&tight, col) - col as f32 * 3.0).abs() <= 1.0);
                    assert_eq!(hit(&loose, x(&loose, col)), col);
                }
            }
        }).textures_delta.clear();
    }

    #[test]
    fn matching_syntax_spans_shape_together_and_tabs_keep_four_columns() {
        let ctx = Context::default();
        ctx.run_ui(Default::default(), |ui| {
            let fmt = eframe::egui::TextFormat::simple(FontId::monospace(16.0), Color32::WHITE);
            let mut job = LayoutJob::default();
            for text in ["=", ">", "\t"] { job.append(text, 0.0, fmt.clone()); }
            assert_eq!(layout(job.clone(), true, 0.0).sections.len(), 1);
            for ligatures in [false, true] {
                for spacing in [-2.0, 2.0] {
                    let end = ui.painter().layout_job(layout(job.clone(), ligatures, spacing));
                    let mut extended = job.clone();
                    extended.append("x", 0.0, fmt.clone());
                    let inside = ui.painter().layout_job(layout(extended, ligatures, spacing));
                    assert!((x(&end, 3) - x(&inside, 3)).abs() <= 1.0);
                }
            }
        }).textures_delta.clear();
    }
}
