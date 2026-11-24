use crate::editor::Encoding;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    pub spaces: Option<bool>,
    pub size: Option<usize>,
    pub crlf: Option<bool>,
    pub charset: Option<Encoding>,
    pub trim: Option<bool>,
    pub final_newline: Option<bool>,
}

impl Props {
    pub fn indent(&self, detected: Option<(bool, usize)>) -> Option<(bool, usize)> {
        match (self.spaces, self.size) {
            (None, None) => detected,
            (spaces, size) => {
                // shortcut: a size with no style and nothing detected means spaces, upgrade to reading insert_spaces if that bites
                let spaces = spaces.or(detected.map(|d| d.0)).unwrap_or(true);
                Some((spaces, size.or(detected.map(|d| d.1).filter(|_| spaces)).unwrap_or(0)))
            }
        }
    }
}

pub fn lookup(path: &Path) -> Props {
    let mut files = vec![];
    for dir in path.ancestors().skip(1) {
        let Ok(text) = std::fs::read_to_string(dir.join(".editorconfig")) else { continue };
        let root = parse(&text).0;
        files.push((dir, text));
        if root {
            break;
        }
    }
    let mut keys = HashMap::new();
    for (dir, text) in files.iter().rev() {
        let Ok(relative) = path.strip_prefix(dir) else { continue };
        let relative = relative.to_string_lossy().replace('\\', "/");
        for (glob, props) in parse(text).1 {
            if matches(&glob, &relative) {
                keys.extend(props);
            }
        }
    }
    props(&keys)
}

type Section = (String, Vec<(String, String)>);

fn parse(text: &str) -> (bool, Vec<Section>) {
    let (mut root, mut sections) = (false, vec![]);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(['#', ';'])) {
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((glob.to_owned(), vec![]));
        } else if let Some((key, value)) = line.split_once('=') {
            let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
            match sections.last_mut() {
                Some((_, props)) => props.push((key, value)),
                None => root |= key == "root" && value == "true",
            }
        }
    }
    (root, sections)
}

// shortcut: {n..m} ranges and backslash escapes are not supported, upgrade if a project needs them
fn matches(glob: &str, relative: &str) -> bool {
    let glob = match glob.strip_prefix('/') {
        Some(anchored) => anchored.to_owned(),
        None if glob.contains('/') => glob.to_owned(),
        None => format!("**/{glob}"),
    };
    crate::glob::glob_match(&glob, relative)
}

fn props(keys: &HashMap<String, String>) -> Props {
    let get = |key: &str| keys.get(key).map(String::as_str).filter(|v| *v != "unset");
    let flag = |key: &str| get(key).and_then(|v| v.parse().ok());
    let tab_width = get("tab_width").and_then(|v| v.parse().ok());
    let size = match get("indent_size") {
        Some("tab") => tab_width,
        Some(v) => v.parse().ok(),
        None => None,
    };
    let spaces = get("indent_style").and_then(|v| match v {
        "space" => Some(true),
        "tab" => Some(false),
        _ => None,
    });
    Props {
        spaces,
        size: if spaces == Some(false) { tab_width.or(size) } else { size.or(tab_width.filter(|_| spaces.is_some())) },
        crlf: get("end_of_line").and_then(|v| match v {
            "crlf" => Some(true),
            "lf" => Some(false),
            _ => None,
        }),
        charset: get("charset").and_then(|v| match v {
            "utf-8" => Some(Encoding::Utf8),
            "utf-8-bom" => Some(Encoding::Utf8Bom),
            "utf-16le" => Some(Encoding::Utf16Le),
            "utf-16be" => Some(Encoding::Utf16Be),
            "latin1" => Some(Encoding::Latin1),
            _ => None,
        }),
        trim: flag("trim_trailing_whitespace"),
        final_newline: flag("insert_final_newline"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearer_files_and_later_sections_win_and_root_stops_the_walk() {
        let base = std::env::temp_dir().join(format!("halo-editorconfig-{}", std::process::id()));
        let sub = base.join("proj").join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join(".editorconfig"), "[*]\nindent_style = tab\n").unwrap();
        std::fs::write(base.join("proj").join(".editorconfig"), "root = true\n[*]\nindent_style = space\nindent_size = 4\nend_of_line = lf\ncharset = utf-8-bom\n[*.{rs,toml}]\nindent_size = 2\ninsert_final_newline = true\n[src/*.md]\ntrim_trailing_whitespace = false\n").unwrap();
        std::fs::write(sub.join(".editorconfig"), "[*.rs]\nend_of_line = crlf\ncharset = unset\n").unwrap();
        let rs = lookup(&sub.join("main.rs"));
        assert_eq!(rs, Props { spaces: Some(true), size: Some(2), crlf: Some(true), charset: None, trim: None, final_newline: Some(true) });
        assert_eq!(rs.indent(Some((false, 0))), Some((true, 2)));
        let md = lookup(&sub.join("notes.md"));
        assert_eq!((md.size, md.trim, md.charset), (Some(4), Some(false), Some(Encoding::Utf8Bom)));
        assert_eq!(lookup(&base.join("proj").join("notes.md")).trim, None);
        assert_eq!(lookup(&base.join("other.txt")).indent(None), Some((false, 0)));
        assert_eq!(Props::default().indent(Some((false, 8))), Some((false, 8)));
        let _ = std::fs::remove_dir_all(base);
    }
}
