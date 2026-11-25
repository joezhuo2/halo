use eframe::egui::epaint::text::{ByteIndex, LayoutJob};

pub(super) fn cells(text: &str) -> (usize, Vec<&str>) {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let mut parts: Vec<&str> = text[indent..].split('\t').collect();
    parts.pop();
    (indent, parts)
}

#[allow(clippy::needless_range_loop)]
pub(super) fn pads(lines: &[(usize, Vec<f32>)], tab: f32) -> Vec<Vec<f32>> {
    let mut out: Vec<Vec<f32>> = lines.iter().map(|l| vec![0.0; l.1.len()]).collect();
    let columns = lines.iter().map(|l| l.1.len()).max().unwrap_or(0);
    for k in 0..columns {
        let mut i = 0;
        while i < lines.len() {
            if lines[i].1.len() <= k {
                i += 1;
                continue;
            }
            let start = i;
            while i + 1 < lines.len() && lines[i + 1].1.len() > k && lines[i + 1].0 == lines[start].0 {
                i += 1;
            }
            let width = (start..=i).map(|j| lines[j].1[k] + tab).fold(0.0, f32::max);
            for j in start..=i {
                out[j][k] = width - lines[j].1[k] - tab;
            }
            i += 1;
        }
    }
    out
}

pub(super) fn apply(mut job: LayoutJob, pads: &[f32]) -> LayoutJob {
    let indent = job.text.len() - job.text.trim_start_matches([' ', '\t']).len();
    let mut at: Vec<(usize, f32)> = job.text[indent..].match_indices('\t').map(|(i, _)| indent + i + 1).zip(pads.iter().copied()).collect();
    at.retain(|a| a.1 > 0.0);
    if at.is_empty() { return job }
    let mut sections = Vec::with_capacity(job.sections.len() + at.len());
    for mut section in std::mem::take(&mut job.sections) {
        if let Some(a) = at.iter().find(|a| a.0 == section.byte_range.start.0) { section.leading_space += a.1; }
        let range = section.byte_range.start.0 + 1..section.byte_range.end.0;
        for &(byte, pad) in at.iter().filter(|a| range.contains(&a.0)) {
            let mut head = section.clone();
            head.byte_range.end = ByteIndex(byte);
            sections.push(head);
            section.byte_range.start = ByteIndex(byte);
            section.leading_space = pad;
        }
        sections.push(section);
    }
    job.sections = sections;
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cells_after_the_indent() {
        assert_eq!(cells("\t\ta\tbb\tc"), (2, vec!["a", "bb"]));
        assert_eq!(cells("plain"), (0, vec![]));
        assert_eq!(cells("x\t"), (0, vec!["x"]));
    }

    #[test]
    fn blocks_align_columns_and_break_on_gaps_and_indent() {
        let lines = vec![(0, vec![10.0, 5.0]), (0, vec![30.0]), (0, vec![]), (0, vec![20.0]), (1, vec![5.0])];
        let pads = pads(&lines, 4.0);
        assert_eq!(pads[0], [20.0, 0.0]);
        assert_eq!(pads[1], [0.0]);
        assert!(pads[2].is_empty());
        assert_eq!(pads[3], [0.0]);
        assert_eq!(pads[4], [0.0]);
    }

    #[test]
    fn second_column_stops_follow_the_first() {
        let lines = vec![(0, vec![10.0, 1.0]), (0, vec![2.0, 8.0])];
        let pads = pads(&lines, 2.0);
        assert_eq!(pads, [vec![0.0, 7.0], vec![8.0, 0.0]]);
    }

    #[test]
    fn apply_pads_the_section_after_each_cell_tab() {
        use eframe::egui::{Color32, FontId, TextFormat};
        let mut job = LayoutJob::default();
        job.append("\tab\tc", 0.0, TextFormat::simple(FontId::monospace(12.0), Color32::WHITE));
        job.append("\td", 0.0, TextFormat::simple(FontId::monospace(12.0), Color32::RED));
        let job = apply(job, &[3.0, 5.0]);
        let starts: Vec<(usize, f32)> = job.sections.iter().map(|s| (s.byte_range.start.0, s.leading_space)).collect();
        assert_eq!(starts, [(0, 0.0), (4, 3.0), (5, 0.0), (6, 5.0)]);
    }
}
