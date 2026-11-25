use crate::app::Halo;
use crate::command::Command;
use crate::layers::{self, Layer};
use crate::settings::{Settings, ShellProfile, Task};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Ui, UiBuilder, pos2, text::LayoutJob, vec2,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const TABS: [(&str, &str); 7] = [
    (icons::TEXT_AA, "Text Editor"),
    (icons::FLOPPY_DISK, "Files"),
    (icons::PALETTE, "Appearance"),
    (icons::TERMINAL_WINDOW, "Terminal"),
    (icons::PLAY, "Run and Tasks"),
    (icons::CODE, "Languages"),
    (icons::KEYBOARD, "Keyboard Shortcuts"),
];

#[derive(Clone, Copy)]
pub enum Kind {
    Toggle,
    Int(i64, i64),
    Float(f64, f64),
    Choice(&'static [(&'static str, &'static str)]),
    Custom,
}

pub struct Meta {
    pub key: &'static str,
    pub tab: usize,
    pub group: &'static str,
    pub title: &'static str,
    pub about: &'static str,
    pub kind: Kind,
}

const fn m(key: &'static str, tab: usize, group: &'static str, title: &'static str, about: &'static str, kind: Kind) -> Meta {
    Meta { key, tab, group, title, about, kind }
}

const ED: &str = "Editor";
const PROBLEMS: &str = "Problems";
const FILES: &str = "Files";
const LANG: &str = "Languages";
const A11Y: &str = "Accessibility";
const THEMES: &[(&str, &str)] = &[("dark", "Dark"), ("light", "Light"), ("hc-dark", "High Contrast Dark"), ("hc-light", "High Contrast Light")];
const SIGNAL: &[(&str, &str)] = &[("auto", "Auto"), ("on", "On"), ("off", "Off")];

pub const SETTINGS: &[Meta] = &[
    m("font_family", 0, ED, "Font Family", "The font of the editor and the terminal.", Kind::Custom),
    m("font_ligatures", 0, ED, "Font Ligatures", "Use programming ligatures when the editor font supports them.", Kind::Toggle),
    m("letter_spacing", 0, ED, "Letter Spacing", "Extra space between editor characters in pixels.", Kind::Float(-5.0, 20.0)),
    m("cursor_smooth_caret_animation", 0, ED, "Smooth Caret Animation", "Animate caret movement. Explicit animates navigation only.", Kind::Choice(&[("off", "Off"), ("explicit", "Explicit"), ("on", "On")])),
    m("copy_with_syntax_highlighting", 0, ED, "Copy With Syntax Highlighting", "Include syntax colours when copying text on Windows.", Kind::Toggle),
    m("font_size", 0, ED, "Font Size", "The font size in pixels.", Kind::Float(8.0, 32.0)),
    m("tab_size", 0, ED, "Tab Size", "The number of spaces a tab is equal to.", Kind::Int(1, 8)),
    m("large_mb", 0, ED, "Large File Size (MB)", "Files above this size open without highlighting, language servers, minimap, git gutter, blame, word wrap and local history.", Kind::Int(1, 250)),
    m("insert_spaces", 0, ED, "Insert Spaces", "Insert spaces when pressing Tab.", Kind::Toggle),
    m("word_wrap", 0, ED, "Word Wrap", "How long lines wrap (Alt+Z toggles).", Kind::Choice(&[("off", "Off"), ("on", "At the editor width"), ("wordWrapColumn", "At the wrap column"), ("bounded", "At the editor width or the wrap column, whichever is less")])),
    m("word_wrap_column", 0, ED, "Word Wrap Column", "The column lines wrap at when Word Wrap uses the wrap column.", Kind::Int(1, 1000)),
    m("line_numbers", 0, ED, "Line Numbers", "How line numbers are shown in the gutter.", Kind::Choice(&[("on", "On"), ("off", "Off"), ("relative", "Relative to the cursor")])),
    m("cursor_style", 0, ED, "Cursor Style", "The shape of the cursor.", Kind::Choice(&[("line", "Line"), ("line-thin", "Thin line"), ("block", "Block"), ("block-outline", "Block outline"), ("underline", "Underline"), ("underline-thin", "Thin underline")])),
    m("cursor_blinking", 0, ED, "Cursor Blinking", "How the cursor blinks.", Kind::Choice(&[("blink", "Blink"), ("smooth", "Smooth"), ("phase", "Phase"), ("expand", "Expand"), ("solid", "Solid")])),
    m("line_height", 0, ED, "Line Height", "0 uses the default. Values below 8 multiply the font size; larger values are pixels.", Kind::Float(0.0, 100.0)),
    m("render_line_highlight", 0, ED, "Render Line Highlight", "Where the line of the cursor is highlighted.", Kind::Choice(&[("none", "None"), ("gutter", "Gutter"), ("line", "Line"), ("all", "Gutter and line")])),
    m("scroll_beyond_last_line", 0, ED, "Scroll Beyond Last Line", "Let the editor scroll half a page past the last line.", Kind::Toggle),
    m("smooth_scrolling", 0, ED, "Smooth Scrolling", "Animate scrolling to the cursor and to search results.", Kind::Toggle),
    m("cursor_surrounding_lines", 0, ED, "Cursor Surrounding Lines", "The number of lines kept visible above and below the cursor when it moves.", Kind::Int(0, 50)),
    m("peek_context_lines", 0, ED, "Peek Context Lines", "The number of lines shown above the target in the peek definition and references preview. The preview scrolls with the mouse wheel.", Kind::Int(1, 40)),
    m("mouse_wheel_zoom", 0, ED, "Mouse Wheel Zoom", "Zoom with Ctrl and the mouse wheel over the editor.", Kind::Toggle),
    m("multi_cursor_modifier", 0, ED, "Multi Cursor Modifier", "The key held while clicking to add a cursor. The other one follows links.", Kind::Choice(&[("alt", "Alt"), ("ctrlCmd", "Ctrl")])),
    m("vim", 0, ED, "Vim Mode", "Edit with Vim keys: normal, insert and visual modes, motions, operators, counts, registers, search and : commands.", Kind::Toggle),
    m("column_selection", 0, ED, "Column Selection", "Dragging with the mouse selects a box of columns instead of a range of text.", Kind::Toggle),
    m("word_separators", 0, ED, "Word Separators", "Characters that end a word for word motions, double click and word deletion. ASCII only.", Kind::Custom),
    m("match_brackets", 0, ED, "Match Brackets", "Highlight the bracket that matches the one at the cursor.", Kind::Choice(&[("always", "Always"), ("near", "Near"), ("never", "Never")])),
    m("selection_highlight", 0, ED, "Selection Highlight", "Highlight other places of the selected text in the visible lines.", Kind::Toggle),
    m("detect_indentation", 0, ED, "Detect Indentation", "Take the tab size and spaces or tabs from the contents of a file when it opens.", Kind::Toggle),
    m("virtual_space", 0, ED, "Virtual Space", "Let the cursor and column selections go past the end of a line. Typing there fills the gap with spaces or tabs.", Kind::Toggle),
    m("auto_close", 0, ED, "Auto Closing Brackets", "Close brackets as they are typed and type over the closing one.", Kind::Toggle),
    m("auto_close_quotes", 0, ED, "Auto Closing Quotes", "Close quotes as they are typed and type over the closing one.", Kind::Toggle),
    m("auto_surround", 0, ED, "Auto Surround", "Wrap the selection when a bracket or quote is typed.", Kind::Toggle),
    m("auto_indent", 0, ED, "Auto Indent", "How Enter indents the new line.", Kind::Choice(&[("none", "None"), ("keep", "Keep the indentation"), ("brackets", "Brackets"), ("advanced", "Advanced"), ("full", "Full")])),
    m("render_whitespace", 0, ED, "Render Whitespace", "Which spaces are drawn as dots and tabs as arrows.", Kind::Choice(&[("none", "None"), ("boundary", "All but single spaces between words"), ("selection", "In the selection"), ("trailing", "Trailing"), ("all", "All")])),
    m("sticky_scroll", 0, ED, "Sticky Scroll", "Keep the lines that open the current scopes at the top while scrolling.", Kind::Toggle),
    m("bracket_colors", 0, ED, "Bracket Pair Colors", "Color brackets by how deeply they are nested.", Kind::Toggle),
    m("guides", 0, ED, "Guides", "Draw indentation guides and highlight the guide of the bracket pair around the cursor.", Kind::Toggle),
    m("diff_side_by_side", 0, ED, "Side by Side Diff", "Open changes in two columns. Off shows one column with the removed lines above the added ones.", Kind::Toggle),
    m("rulers", 0, ED, "Rulers", "Columns that get a vertical line, separated by commas.", Kind::Custom),
    m("minimap", 0, ED, "Minimap", "Show the minimap beside the editor.", Kind::Toggle),
    m("minimap_side", 0, ED, "Minimap Side", "The side of the editor the minimap is on.", Kind::Choice(&[("right", "Right"), ("left", "Left")])),
    m("breadcrumbs", 0, ED, "Breadcrumbs", "Show the path and the enclosing symbols above the text.", Kind::Toggle),
    m("hide_secrets", 0, ED, "Hide Secrets on Screen", "Mask credential values until a caret or selection touches their line.", Kind::Toggle),
    m("code_lens", 0, ED, "Code Lens", "Show reference counts and the language server's lenses, such as Run and Debug, above declarations. Click one to run it.", Kind::Toggle),
    m("lightbulb", 0, ED, "Lightbulb", "Show a lightbulb in the gutter when code actions are available on the cursor line.", Kind::Toggle),
    m("inlay_hints", 0, ED, "Inlay Hints", "Show inferred types and parameter names inside the line.", Kind::Toggle),
    m("color_decorators", 0, ED, "Color Decorators", "Show a swatch before each color value. Click it to pick a new color.", Kind::Toggle),
    m("unicode_invisible", 0, ED, "Highlight Invisible Characters", "Draw a box around zero width and other invisible characters.", Kind::Toggle),
    m("unicode_ambiguous", 0, ED, "Highlight Ambiguous Characters", "Draw a box around characters that look like ASCII ones, such as a Cyrillic a. Not in plain text or Markdown.", Kind::Toggle),
    m("semantic_highlighting", 0, ED, "Semantic Highlighting", "Color types, functions and constants as the language server reports them.", Kind::Toggle),
    m("suggest_rows", 0, ED, "Suggestion Rows", "The number of suggestions the completion list shows before it scrolls.", Kind::Int(1, 20)),
    m("inline_blame", 0, ED, "Inline Blame", "Show the commit beside the caret line.", Kind::Toggle),
    m("local_history", 0, ED, "Local History", "Keep up to 50 saved versions of each file in the Timeline.", Kind::Toggle),
    m("format_on_save", 0, ED, "Format On Save", "Format the file with its language server when it is saved by hand.", Kind::Toggle),
    m("format_on_paste", 0, ED, "Format On Paste", "Format pasted text with the language server.", Kind::Toggle),
    m("format_on_type", 0, ED, "Format On Type", "Let the language server reformat the line after you type characters it reacts to, such as a semicolon or a closing brace.", Kind::Toggle),
    m("linked_editing", 0, ED, "Linked Editing", "Rename the matching HTML, XML or JSX tag while you edit a tag name.", Kind::Toggle),
    m("inline_suggest", 0, ED, "Inline Suggestions", "Show an AI completion as dimmed text after the cursor when you pause typing. Tab accepts it. Each request runs the Claude or Codex CLI and uses its quota.", Kind::Toggle),
    m("inline_edits", 0, ED, "Next Edit Suggestions", "After you edit and pause, ask the AI for the next likely edit in the file. Tab jumps to it and Tab again applies it. Each request uses the CLI's quota.", Kind::Toggle),
    m("diagnostics", 0, PROBLEMS, "Diagnostics", "Underline errors and warnings, and list them in the Problems panel.", Kind::Toggle),
    m("status_problems", 0, PROBLEMS, "Status Bar Counts", "Show the number of errors and warnings in the status bar.", Kind::Toggle),
    m("markdown_validate", 0, PROBLEMS, "Validate Markdown Links", "Warn about links in Markdown files to files, headings and link definitions that do not exist.", Kind::Toggle),
    m("auto_save", 1, FILES, "Auto Save", "When changed files are saved.", Kind::Choice(&[("off", "Off"), ("afterDelay", "After a delay"), ("onFocusChange", "When the editor loses focus"), ("onWindowChange", "When the window loses focus")])),
    m("auto_save_delay", 1, FILES, "Auto Save Delay", "Milliseconds after the last edit before a file is saved, when Auto Save is After a delay.", Kind::Int(100, 60000)),
    m("hot_exit", 1, FILES, "Hot Exit", "Keep unsaved and untitled files when Halo closes and bring them back the next time it opens.", Kind::Toggle),
    m("persistent_undo", 1, FILES, "Persistent Undo", "Keep the undo history of saved files after they are closed and Halo restarts. It is dropped when the file changes outside Halo.", Kind::Toggle),
    m("trim_trailing_whitespace", 1, FILES, "Trim Trailing Whitespace", "Remove spaces and tabs at the ends of lines when a file is saved.", Kind::Toggle),
    m("insert_final_newline", 1, FILES, "Insert Final Newline", "End the file with a line break when it is saved.", Kind::Toggle),
    m("trim_final_newlines", 1, FILES, "Trim Final Newlines", "Remove the empty lines after the last line break when a file is saved.", Kind::Toggle),
    m("eol", 1, FILES, "End of Line", "The line ending of new files.", Kind::Choice(&[("auto", "Default for the system"), ("\n", "LF"), ("\r\n", "CRLF")])),
    m("auto_update", 1, "Application", "Auto Update", "When Halo opens, look for a newer release, install it in the background and use it the next time Halo opens.", Kind::Toggle),
    m("zoom_level", 2, A11Y, "Zoom Level", "The zoom of the whole window. Each step is 20% larger or smaller; 0 is 100%.", Kind::Float(-4.0, 6.0)),
    m("auto_detect_high_contrast", 2, A11Y, "Auto Detect High Contrast", "Switch to a high contrast theme while the operating system is in high contrast mode (Windows).", Kind::Toggle),
    m("preferred_high_contrast_theme", 2, A11Y, "Preferred High Contrast Theme", "The theme used when the operating system is in a dark high contrast mode.", Kind::Choice(THEMES)),
    m("preferred_high_contrast_light_theme", 2, A11Y, "Preferred High Contrast Light Theme", "The theme used when the operating system is in a light high contrast mode.", Kind::Choice(THEMES)),
    m("accessibility_support", 2, A11Y, "Accessibility Support", "Optimize for screen readers: word wrap, minimap, code lens, inlay hints and animations are turned off. Auto follows whether a screen reader is active.", Kind::Choice(&[("auto", "Auto"), ("on", "On"), ("off", "Off")])),
    m("signal_line_error", 2, A11Y, "Signal: Line Has Error", "Play a sound when the cursor moves onto a line with an error (Windows). Auto plays it only while a screen reader is active.", Kind::Choice(SIGNAL)),
    m("signal_line_warning", 2, A11Y, "Signal: Line Has Warning", "Play a sound when the cursor moves onto a line with a warning and no error (Windows). Auto plays it only while a screen reader is active.", Kind::Choice(SIGNAL)),
    m("signal_line_breakpoint", 2, A11Y, "Signal: Line Has Breakpoint", "Play a sound when the cursor moves onto a line with a breakpoint (Windows). Auto plays it only while a screen reader is active.", Kind::Choice(SIGNAL)),
    m("signal_line_folded", 2, A11Y, "Signal: Line Has Folded Area", "Play a sound when the cursor moves onto a folded line (Windows). Auto plays it only while a screen reader is active.", Kind::Choice(SIGNAL)),
    m("signal_diff_inserted", 2, A11Y, "Signal: Diff Line Inserted", "Play a sound when the accessible diff viewer moves to an inserted line (Windows). Auto plays it only while a screen reader is active.", Kind::Choice(SIGNAL)),
    m("signal_diff_deleted", 2, A11Y, "Signal: Diff Line Deleted", "Play a sound when the accessible diff viewer moves to a removed line (Windows). Auto plays it only while a screen reader is active.", Kind::Choice(SIGNAL)),
    m("signal_volume", 2, A11Y, "Signal Volume", "The volume of the signal sounds, from 0 to 100.", Kind::Int(0, 100)),
    m("reduce_motion", 2, A11Y, "Reduce Motion", "Jump animations to their end, stop loading shimmers and spinners, and stop the cursor blinking. Auto follows the operating system (Windows).", Kind::Choice(&[("auto", "Auto"), ("on", "On"), ("off", "Off")])),
    m("restore_session", 2, "Window", "Restore Session", "Open the last folder and its files when Halo starts.", Kind::Toggle),
    m("tab_close_button", 2, "Workbench", "Tab Close Button", "Where the close button of an editor tab is.", Kind::Choice(&[("right", "Right"), ("left", "Left"), ("off", "Off")])),
    m("enable_preview", 2, "Workbench", "Enable Preview", "Open files from a single click in a preview tab that the next one replaces.", Kind::Toggle),
    m("wrap_tabs", 2, "Workbench", "Wrap Tabs", "Wrap editor tabs onto more rows when they do not fit in one, instead of scrolling them.", Kind::Toggle),
    m("sidebar_location", 2, "Workbench", "Side Bar Location", "The side of the window the primary side bar and the activity bar are on. The secondary side bar is on the other side.", Kind::Choice(&[("left", "Left"), ("right", "Right")])),
    m("activity_bar_location", 2, "Workbench", "Activity Bar Location", "Where the icons that switch the views of the primary side bar are.", Kind::Choice(&[("default", "Beside the side bar"), ("top", "At the top of the side bar"), ("hidden", "Hidden")])),
    m("panel_location", 2, "Workbench", "Panel Default Location", "Where the panel opens. View: Move Panel changes it for the session.", Kind::Choice(&[("bottom", "Bottom"), ("top", "Top"), ("left", "Left"), ("right", "Right")])),
    m("panel_alignment", 2, "Workbench", "Panel Alignment", "How far a panel at the bottom reaches under the side bars.", Kind::Choice(&[("center", "Between the side bars"), ("justify", "Under both side bars"), ("left", "Under the left side bar"), ("right", "Under the right side bar")])),
    m("zen_full_screen", 2, "Zen Mode", "Full Screen", "Go full screen when zen mode starts.", Kind::Toggle),
    m("zen_center_layout", 2, "Zen Mode", "Center Layout", "Center the editor when zen mode starts.", Kind::Toggle),
    m("zen_show_tabs", 2, "Zen Mode", "Show Tabs", "Whether editor tabs are shown in zen mode.", Kind::Choice(&[("multiple", "Show"), ("none", "Hide")])),
    m("zen_hide_line_numbers", 2, "Zen Mode", "Hide Line Numbers", "Hide line numbers in zen mode.", Kind::Toggle),
    m("zen_restore", 2, "Zen Mode", "Restore", "Start in zen mode when Halo was closed in zen mode.", Kind::Toggle),
    m("compact_folders", 1, "Explorer", "Compact Folders", "Show a chain of folders that each hold only one folder as a single row in the Explorer.", Kind::Toggle),
    m("confirm_drag_and_drop", 1, "Explorer", "Confirm Drag And Drop", "Ask before moving files and folders by dragging them in the Explorer.", Kind::Toggle),
    m("file_nesting", 1, "Explorer", "File Nesting", "Show related files, such as a lock file or compiled output, nested under the file they belong to.", Kind::Toggle),
    m("file_nesting_expand", 1, "Explorer", "File Nesting Expand", "Start nested groups expanded.", Kind::Toggle),
    m("file_nesting_patterns", 1, "Explorer", "File Nesting Patterns", "Files matching the patterns on the right nest under a file matching the one on the left. The left pattern may have one *, used as ${capture}; the right ones may use ${capture}, ${basename}, ${extname}, ${dirname} and *.", Kind::Custom),
    m("files_exclude", 1, FILES, "Exclude", "Glob patterns of files and folders the Explorer hides. Supports *, **, ?, [abc] and {a,b}.", Kind::Custom),
    m("confirm_delete", 2, "Explorer", "Confirm Delete", "Ask before moving a file to the recycle bin from the Explorer.", Kind::Toggle),
    m("explorer_sort", 2, "Explorer", "Sort Order", "How the Explorer orders files and folders.", Kind::Choice(&[("default", "Folders first, by name"), ("mixed", "By name"), ("filesFirst", "Files first"), ("type", "Folders first, by extension"), ("modified", "Folders first, newest first")])),
    m("theme", 2, "Workbench", "Color Theme", "The base theme. Custom colors are kept for each theme. Import more with Preferences: Import VS Code Theme.", Kind::Custom),
    m("icon_theme", 2, "Workbench", "File Icon Theme", "The icons shown next to files and folders. Import more with Preferences: Import VS Code Icon Theme.", Kind::Custom),
    m("window_color", 2, "Workbench", "Window Color", "Tint the title bar so this window is easy to tell apart. Set it in the folder settings to give each folder its own color; Window: Set Color... does that.", Kind::Choice(crate::titlebar::WINDOW_COLORS)),
    m("show_tabs", 2, "Workbench", "Show Tabs", "Whether editor groups show their tab strip. Hide it to use the Open Editors list in the Explorer as vertical tabs.", Kind::Choice(&[("multiple", "Show"), ("none", "Hide")])),
    m("open_editors_tree", 2, "Workbench", "Open Editors Tree", "Group the Open Editors list in the Explorer by folder.", Kind::Toggle),
    m("colors", 2, "Workbench", "Colors", "Interface colors on the left, syntax colors on the right.", Kind::Custom),
    m("default_terminal", 3, "Terminal", "Default Profile", "The shell a new terminal starts with.", Kind::Custom),
    m("terminals", 3, "Terminal", "Profiles", "The name, program and arguments of each shell.", Kind::Custom),
    m("shell_integration", 3, "Terminal", "Shell Integration", "Let PowerShell, bash and zsh report their prompts, commands, exit codes and folder, for command marks and navigation.", Kind::Toggle),
    m("terminal_font_family", 3, "Terminal", "Font Family", "The font of the terminals. Empty uses the editor font.", Kind::Custom),
    m("terminal_font_size", 3, "Terminal", "Font Size", "The font size of the terminals in pixels. 0 uses the editor font size.", Kind::Float(0.0, 32.0)),
    m("terminal_line_height", 3, "Terminal", "Line Height", "The height of a terminal row as a multiple of the font height.", Kind::Float(1.0, 3.0)),
    m("terminal_cursor_style", 3, "Terminal", "Cursor Style", "The cursor shape when the program in the terminal does not choose one.", Kind::Choice(&[("block", "Block"), ("line", "Line"), ("underline", "Underline")])),
    m("terminal_cursor_blinking", 3, "Terminal", "Cursor Blinking", "Let the terminal cursor blink.", Kind::Toggle),
    m("terminal_scrollback", 3, "Terminal", "Scrollback", "The number of lines a new terminal keeps above the screen.", Kind::Int(100, 100000)),
    m("send_keybindings_to_shell", 3, "Terminal", "Send Keybindings to Shell", "Let Ctrl+B, Ctrl+J, Ctrl+K, Ctrl+N, Ctrl+O, Ctrl+S and Ctrl+W reach the shell while a terminal has focus.", Kind::Toggle),
    m("terminal_persistent_sessions", 3, "Terminal", "Persistent Sessions", "Save the terminals of the workspace when Halo exits and recreate them with their history when it starts again.", Kind::Toggle),
    m("terminal_persistent_scrollback", 3, "Terminal", "Persistent Session Scrollback", "The number of lines of each terminal that are saved and restored.", Kind::Int(0, 100000)),
    m("terminal_copy_on_selection", 3, "Terminal", "Copy On Selection", "Copy text to the clipboard as soon as it is selected in a terminal.", Kind::Toggle),
    m("terminal_right_click", 3, "Terminal", "Right Click Behavior", "What a right click in a terminal does.", Kind::Choice(&[("default", "Nothing (default)"), ("copyPaste", "Copy the selection, or paste without one"), ("paste", "Paste"), ("selectWord", "Select the word"), ("nothing", "Nothing")])),
    m("terminal_suggest", 3, "Terminal", "Suggest", "Suggest commands, subcommands, flags, folders and files as you type in a terminal with shell integration. Tab accepts, Ctrl+Space opens the list.", Kind::Toggle),
    m("terminal_sticky_scroll", 3, "Terminal", "Sticky Scroll", "Keep the command whose output fills the terminal pinned to its top row.", Kind::Toggle),
    m("terminal_images", 3, "Terminal", "Images", "Show Sixel and iTerm inline images that programs print in a terminal. Not available on Windows, whose pseudo console drops them.", Kind::Toggle),
    m("run", 4, "Run", "Commands", "The command Run File uses for each file extension. {file} is replaced by the path of the file.", Kind::Custom),
    m("tasks", 4, "Run", "Tasks", "Named commands that are listed in the Run menu.", Kind::Custom),
    m("grammars", 5, LANG, "Grammars Folder", "A folder of .sublime-syntax grammars. New grammars and associations are loaded when Halo starts.", Kind::Custom),
    m("associations", 5, LANG, "File Associations", "Open files with an extension as another language, given by its name or its extension.", Kind::Custom),
    m("lsp", 5, "Language Servers", "Enabled", "Start the language server of a file for diagnostics, completion, hover, navigation, rename and formatting.", Kind::Toggle),
    m("servers", 5, "Language Servers", "Commands", "The command that starts the server for each file extension. It must be on PATH and speak LSP over standard input and output.", Kind::Custom),
    m("inline_values", 5, "Debugging", "Inline Values", "Show variable values beside source lines while debugging is paused.", Kind::Toggle),
    m("debug_adapters", 5, "Debugging", "Debug Adapters", "The command that starts the debug adapter for each launch.json type. {port} makes Halo connect to it over TCP.", Kind::Custom),
];

#[derive(Default)]
pub struct SettingsPage {
    pub open: bool,
    pub keys: crate::shortcuts::KeysPage,
    tab: usize,
    layer: usize,
    query: String,
    focus: bool,
    pub(crate) themes: Option<(Vec<String>, Vec<String>)>,
    confirm: Option<String>,
    fonts: Option<Vec<(&'static str, String)>>,
    new_run: (String, String),
    new_association: (String, String),
    new_server: (String, String),
    new_adapter: (String, String),
    new_nesting: (String, String),
    new_exclude: String,
}

pub struct Query {
    pub modified: bool,
    pub lang: Option<String>,
    pub words: Vec<String>,
}

pub fn parse_query(text: &str) -> Query {
    let mut query = Query { modified: false, lang: None, words: vec![] };
    for part in text.split_whitespace() {
        if part.eq_ignore_ascii_case("@modified") {
            query.modified = true;
        } else if let Some(id) = part.strip_prefix("@lang:") {
            query.lang = Some(id.to_lowercase()).filter(|id| !id.is_empty());
        } else {
            query.words.extend(words(part));
        }
    }
    query
}

fn words(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut word = String::new();
    let mut previous = ' ';
    for c in text.chars() {
        if (!c.is_alphanumeric() || (c.is_uppercase() && previous.is_lowercase())) && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        }
        previous = c;
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn word_score(query: &str, word: &str) -> u32 {
    let subsequence = || {
        let mut chars = word.chars();
        query.chars().all(|q| chars.any(|w| w == q))
    };
    if word == query {
        4
    } else if word.starts_with(query) {
        3
    } else if query.len() >= 3 && word.contains(query) {
        2
    } else if query.len() >= 3 && word.starts_with(&query[..1]) && subsequence() {
        1
    } else {
        0
    }
}

pub fn score(meta: &Meta, query: &[String]) -> Option<u32> {
    let code = layers::vscode_key(meta.key).unwrap_or_default();
    let fields = [(words(meta.title), 3), (words(&format!("{} {code}", meta.key)), 2), (words(meta.group), 1), (words(meta.about), 1)];
    let mut total = 0;
    for q in query {
        let best = fields.iter().flat_map(|(list, weight)| list.iter().map(move |w| word_score(q, w) * weight)).max().unwrap_or(0);
        if best == 0 {
            return None;
        }
        total += best;
    }
    let title = meta.title.to_lowercase();
    Some(total + if !query.is_empty() && title.contains(&query.join(" ")) { 10 } else { 0 })
}

fn highlight(text: &str, query: &[String], color: Color32, accent: Color32, size: f32) -> LayoutJob {
    let lower = text.to_lowercase();
    let mut marked = vec![false; text.len()];
    for q in query.iter().filter(|q| !q.is_empty()) {
        for (at, _) in lower.match_indices(q.as_str()) {
            marked[at..at + q.len()].iter_mut().for_each(|m| *m = true);
        }
    }
    let mut job = LayoutJob::default();
    let mut start = 0;
    for end in (1..=text.len()).filter(|&i| text.is_char_boundary(i)) {
        if end == text.len() || marked.get(end) != marked.get(start) && text.is_char_boundary(end) {
            let format = egui::TextFormat {
                font_id: FontId::proportional(size),
                color: if marked[start] { accent } else { color },
                background: if marked[start] { ui::with_alpha(accent, 0.18) } else { Color32::TRANSPARENT },
                ..Default::default()
            };
            job.append(&text[start..end], 0.0, format);
            start = end;
        }
    }
    job
}

fn to_map(settings: &Settings) -> Map<String, Value> {
    match serde_json::to_value(settings) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

struct View {
    layer: Layer,
    lang: Option<String>,
    defaults: Map<String, Value>,
}

impl View {
    fn modified(&self, key: &str, values: &Map<String, Value>) -> bool {
        match (&self.lang, self.layer) {
            (Some(id), layer) => layers::sets(layer, Some(id), key),
            (None, layer @ (Layer::Workspace | Layer::Folder)) => layers::sets(layer, None, key),
            (None, Layer::User) => values.get(key) != self.defaults.get(key),
        }
    }

    fn notes(&self, key: &str) -> Vec<String> {
        let mut notes = vec![];
        if self.layer == Layer::User && self.lang.is_none() && layers::sets(Layer::Workspace, None, key) {
            notes.push("Overridden by Workspace".to_owned());
        }
        if self.layer != Layer::Folder && self.lang.is_none() && !layers::folders().is_empty() && layers::sets(Layer::Folder, None, key) {
            notes.push("Overridden by Folder".to_owned());
        }
        if self.lang.is_none() {
            let langs: Vec<String> = layers::languages()
                .into_iter()
                .filter(|id| [Layer::User, Layer::Workspace, Layer::Folder].iter().any(|l| layers::sets(*l, Some(id), key)))
                .map(|id| format!("[{id}]"))
                .collect();
            if !langs.is_empty() {
                notes.push(format!("Overridden for {}", langs.join(", ")));
            }
        }
        if self.layer == Layer::User && self.lang.as_ref().is_some_and(|id| layers::sets(Layer::Workspace, Some(id), key)) {
            notes.push("Overridden by Workspace".to_owned());
        }
        notes
    }
}

impl SettingsPage {
    pub fn find_key(&mut self, ctx: &egui::Context) {
        if self.open && self.tab != 6 && ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::F))) {
            self.focus = true;
        }
    }
}

impl Halo {
    pub(crate) fn settings_dialog(&mut self, ctx: &egui::Context) {
        let theme = self.theme.clone();
        let mut close = false;
        let screen = ctx.content_rect();
        let size = vec2((screen.width() - 120.0).clamp(520.0, 920.0), (screen.height() - 120.0).clamp(320.0, 640.0));
        let query = parse_query(&self.page.query);
        let has_folder = self.session.folder.is_some();
        let folders = layers::folders();
        if !has_folder {
            self.page.layer = 0;
        }
        if folders.is_empty() {
            self.page.layer = self.page.layer.min(1);
        }
        let layer = [Layer::User, Layer::Workspace, Layer::Folder][self.page.layer.min(2)];
        let view = View { layer, lang: query.lang.clone(), defaults: to_map(&Settings::default()) };
        let before = layers::scoped(layer, view.lang.as_deref());
        let mut edited = before.clone();
        let mut values = to_map(&before);
        let searching = !self.page.query.trim().is_empty();
        let mut resets = vec![];
        let modal = ui::modal(ctx, "settings_page", ui::Enter::Widget, |ui, _| {
            ui::outline(ui);
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}  Settings", icons::SLIDERS_HORIZONTAL)).size(15.0).color(theme.text));
                ui.add_space(16.0);
                if self.page.tab != 6 {
                    let search = TextEdit::singleline(&mut self.page.query)
                        .id(Id::new("settings_search"))
                        .margin(ui::FIELD)
                        .desired_width(300.0)
                        .hint_text("Search settings (@modified, @lang:rust)");
                    let resp = ui.add(search);
                    if std::mem::take(&mut self.page.focus) {
                        resp.request_focus();
                    }
                    let labels: &[&str] = if folders.is_empty() { &["User", "Workspace"] } else { &["User", "Workspace", "Folder"] };
                    let mut picked = self.page.layer;
                    ui.add_enabled_ui(has_folder, |ui| ui::tabs(ui, &theme, Id::new("settings_layer"), labels, &mut picked))
                        .response
                        .on_disabled_hover_text("Open a folder to edit its .vscode/settings.json");
                    self.page.layer = picked;
                    if layer == Layer::Folder && folders.len() > 1 {
                        let names = crate::workspace::current().names();
                        let mut target = layers::target();
                        let shown = names.get(target).cloned().unwrap_or_default();
                        egui::ComboBox::from_id_salt("settings_folder").selected_text(shown).show_index(ui, &mut target, names.len(), |i| names[i].clone());
                        if target != layers::target() {
                            layers::set_target(target);
                        }
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close |= ui::icon_button(ui, &theme, icons::X, 24.0, false, "Close").clicked();
                    if ui::icon_button(ui, &theme, icons::BRACKETS_CURLY, 24.0, false, "Open settings.json").clicked() {
                        match layers::path(layer).filter(|_| layer != Layer::User) {
                            Some(path) => {
                                if !path.exists() {
                                    crate::settings::write(&path, &json!({}));
                                }
                                self.cmds.push(Command::OpenPath(path));
                            }
                            None => self.cmds.push(Command::OpenSettingsJson),
                        }
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            let body = ui.available_rect_before_wrap();
            ui.allocate_rect(body, Sense::hover());
            ui.painter().hline(body.x_range(), body.top(), Stroke::new(1.0, theme.border));
            let nav = Rect::from_min_max(body.min + vec2(0.0, 12.0), pos2(body.left() + 196.0, body.bottom()));
            ui.painter().vline(nav.right() + 12.0, body.y_range(), Stroke::new(1.0, theme.border));
            ui.scope_builder(UiBuilder::new().max_rect(nav), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, (icon, name)) in TABS.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(nav.width(), 32.0), Sense::click());
                    if resp.clicked() {
                        self.page.tab = index;
                        self.page.keys.stop();
                        if index != 6 {
                            self.page.query.clear();
                        }
                    }
                    let on = ui::anim(ui, resp.id.with("on"), self.page.tab == index && !searching);
                    ui::item(ui, &resp, ui::Role::Tab, name, self.page.tab == index && !searching);
                    let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
                    let fill = ui::mix(ui::mix(Color32::TRANSPARENT, theme.hover, h), theme.selection, on);
                    ui.painter().rect_filled(r, CornerRadius::same(5), fill);
                    ui::focus_ring(ui, &theme, r, resp.id);
                    let color = ui::mix(theme.text_dim, theme.text, on.max(h));
                    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, *icon, FontId::proportional(15.0), color);
                    ui.painter().text(pos2(r.left() + 38.0, r.center().y), Align2::LEFT_CENTER, *name, FontId::proportional(13.0), color);
                }
            });
            let content = Rect::from_min_max(pos2(nav.right() + 34.0, body.top() + 1.0), body.max);
            ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
                ui.set_clip_rect(content.intersect(ui.clip_rect()));
                let salt = if searching { 99 } else { self.page.tab };
                ScrollArea::both().id_salt(("settings_tab", salt)).auto_shrink(false).show(ui, |ui| {
                    ui.set_width(ui.available_width() - 16.0);
                    ui.add_space(14.0);
                    if self.page.tab == 6 && !searching {
                        ui.label(RichText::new(TABS[6].1).size(22.0).color(theme.text));
                        close |= crate::shortcuts::page(ui, &theme, &mut self.page.keys, &mut self.cmds);
                        return;
                    }
                    let mut shown: Vec<(&Meta, u32)> = SETTINGS
                        .iter()
                        .filter(|meta| searching || meta.tab == self.page.tab)
                        .filter(|meta| !query.modified || view.modified(meta.key, &values))
                        .filter(|meta| view.lang.as_ref().is_none_or(|_| !query.words.is_empty() || query.modified || layers::sets(Layer::User, view.lang.as_deref(), meta.key) || layers::sets(Layer::Workspace, view.lang.as_deref(), meta.key)))
                        .filter_map(|meta| score(meta, &query.words).map(|s| (meta, s)))
                        .collect();
                    if searching {
                        shown.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
                        let found = match shown.len() {
                            1 => "1 setting found".to_owned(),
                            n => format!("{n} settings found"),
                        };
                        let scope = match &view.lang {
                            Some(id) => format!("{found} in [{id}] of the {layer:?} settings"),
                            None => found,
                        };
                        ui.label(RichText::new(scope).size(13.0).color(theme.text_dim));
                    } else {
                        ui.label(RichText::new(TABS[self.page.tab].1).size(22.0).color(theme.text));
                    }
                    for (meta, _) in shown {
                        if let Some(key) = row(ui, &theme, meta, &view, &query.words, &mut values, &mut edited, &mut self.page) {
                            resets.push((key.to_owned(), None));
                        }
                    }
                    if self.page.tab == 5 && !searching {
                        installed(ui, &theme);
                    }
                    ui.add_space(18.0);
                });
            });
        });
        let original = to_map(&before);
        let mut edits = layers::changes(&original, &edited);
        edits.extend(values.into_iter().filter(|(k, v)| original.get(k) != Some(v)).map(|(k, v)| (k, Some(v))));
        edits.extend(resets);
        if !edits.is_empty() {
            layers::write(layer, view.lang.as_deref(), &edits);
            self.settings = Settings::load();
            self.apply_settings(ctx);
        }
        if close || modal.should_close() {
            self.page.open = false;
            self.page.keys.stop();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, theme: &Theme, meta: &Meta, view: &View, query: &[String], values: &mut Map<String, Value>, s: &mut Settings, page: &mut SettingsPage) -> Option<&'static str> {
    let modified = view.modified(meta.key, values);
    let mut reset = None;
    ui.add_space(20.0);
    let header = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if modified {
            let (r, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(1), theme.accent);
        }
        ui.label(highlight(&format!("{}:", meta.group), query, theme.text_dim, theme.accent, 13.5));
        ui.label(highlight(meta.title, query, theme.text, theme.accent, 13.5));
        if modified {
            ui.label(RichText::new("Modified").size(11.5).color(theme.accent));
        }
        for note in view.notes(meta.key) {
            ui.label(RichText::new(note).size(11.5).color(theme.text_dim));
        }
        let gear = ui::icon_button(ui, theme, icons::GEAR, 20.0, false, "More actions");
        egui::Popup::menu(&gear).show(|ui| {
            ui::menu_pad(ui);
            if ui.add_enabled(modified, egui::Button::new("Reset to Default")).clicked() {
                reset = Some(meta.key);
                ui.close();
            }
            if ui.button("Copy Setting ID").clicked() {
                ui.ctx().copy_text(layers::vscode_key(meta.key).unwrap_or_else(|| meta.key.to_owned()));
                ui.close();
            }
        });
    });
    let id_text = match layers::vscode_key(meta.key) {
        Some(code) => format!("{} (VS Code: {code})", meta.key),
        None => meta.key.to_owned(),
    };
    header.response.on_hover_text(id_text);
    let toggle = matches!(meta.kind, Kind::Toggle);
    if !toggle && !meta.about.is_empty() {
        ui.add(egui::Label::new(highlight(meta.about, query, theme.text_dim, theme.accent, 13.0)).wrap());
    }
    ui.add_space(5.0);
    let value = values.entry(meta.key.to_owned()).or_insert(Value::Null);
    match meta.kind {
        Kind::Toggle => {
            let mut on = value.as_bool().unwrap_or(false);
            if ui::checkbox(ui, theme, &mut on, highlight(meta.about, query, theme.text_dim, theme.accent, 13.0)).changed() {
                *value = json!(on);
            }
        }
        Kind::Int(min, max) => {
            let mut n = value.as_i64().unwrap_or(min);
            if ui.add(egui::DragValue::new(&mut n).range(min..=max).speed(0.1)).changed() {
                *value = json!(n);
            }
        }
        Kind::Float(min, max) => {
            let mut n = value.as_f64().unwrap_or(min);
            if ui.add(egui::DragValue::new(&mut n).range(min..=max).speed(0.25)).changed() {
                *value = json!(n);
            }
        }
        Kind::Choice(choices) => {
            let current = value.as_str().unwrap_or_default().to_owned();
            let shown = choices.iter().find(|c| c.0 == current).map_or(current.clone(), |c| c.1.to_owned());
            egui::ComboBox::from_id_salt(("setting", meta.key)).width(240.0).selected_text(shown).show_ui(ui, |ui| {
                for (option, label) in choices {
                    if ui.selectable_label(current == *option, *label).clicked() {
                        *value = json!(option);
                    }
                }
            });
        }
        Kind::Custom => custom(ui, theme, meta.key, s, page),
    }
    reset
}

fn field(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> egui::Response {
    ui.add(TextEdit::singleline(text).margin(ui::FIELD).desired_width(width).min_size(vec2(width, 0.0)).hint_text(hint))
}

fn grid<R>(ui: &mut Ui, id: &str, columns: usize, rows: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id).num_columns(columns).spacing(vec2(8.0, 6.0)).show(ui, rows).inner
}

fn remove_button(ui: &mut Ui, theme: &Theme) -> bool {
    ui::icon_button(ui, theme, icons::TRASH, 24.0, false, "Remove").clicked()
}

fn custom(ui: &mut Ui, theme: &Theme, key: &str, s: &mut Settings, page: &mut SettingsPage) {
    match key {
        "font_family" => font_family(ui, &mut s.font_family, "Default (Hack)", page),
        "terminal_font_family" => font_family(ui, &mut s.terminal_font_family, "Editor font", page),
        "rulers" => rulers(ui, s),
        "word_separators" => drop(field(ui, &mut s.word_separators, 320.0, "separators")),
        "colors" => colors(ui, theme, s),
        "theme" | "icon_theme" => {
            let (themes, icon_themes) = page
                .themes
                .get_or_insert_with(|| {
                    let config = crate::settings::dir();
                    (crate::theme_import::installed(&config), crate::icon_theme::installed(&config))
                })
                .clone();
            let removed = match key {
                "theme" => choose(ui, "color_theme", &mut s.theme, THEMES, &themes, &mut page.confirm, crate::theme_import::remove),
                _ => choose(ui, "icon_theme", &mut s.icon_theme, &[("builtin", "Halo (default)"), ("none", "None")], &icon_themes, &mut page.confirm, crate::icon_theme::remove),
            };
            if removed {
                page.themes = None;
            }
        }
        "files_exclude" => exclude_editor(ui, theme, &mut s.files_exclude, &mut page.new_exclude),
        "default_terminal" => {
            let names: Vec<String> = s.terminals.iter().map(|p| p.name.clone()).collect();
            egui::ComboBox::from_id_salt("default_terminal").width(240.0).selected_text(s.default_terminal.clone()).show_ui(ui, |ui| {
                for name in names {
                    ui.selectable_value(&mut s.default_terminal, name.clone(), name);
                }
            });
        }
        "terminals" => profiles(ui, theme, s),
        "run" => map_editor(ui, theme, "settings_run", &mut s.run, &mut page.new_run, "command"),
        "tasks" => tasks(ui, theme, s),
        "grammars" => grammars(ui, s),
        "associations" => map_editor(ui, theme, "settings_associations", &mut s.associations, &mut page.new_association, "language"),
        "servers" => map_editor(ui, theme, "settings_servers", &mut s.servers, &mut page.new_server, "command"),
        "debug_adapters" => map_editor(ui, theme, "settings_adapters", &mut s.debug_adapters, &mut page.new_adapter, "command"),
        "file_nesting_patterns" => map_editor(ui, theme, "settings_nesting", &mut s.file_nesting_patterns, &mut page.new_nesting, "child patterns"),
        _ => {}
    }
}

fn font_family(ui: &mut Ui, value: &mut String, empty: &str, page: &mut SettingsPage) {
    let fonts = page.fonts.get_or_insert_with(crate::settings::font_files);
    let shown = match fonts.iter().find(|font| font.1 == *value) {
        Some(font) => font.0.to_owned(),
        None if value.is_empty() => empty.to_owned(),
        None => Path::new(value.as_str()).file_name().unwrap_or_default().to_string_lossy().into_owned(),
    };
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("font_family", empty)).width(240.0).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(value, String::new(), empty);
            for (name, file) in fonts.iter() {
                ui.selectable_value(value, file.clone(), *name);
            }
        });
        if ui.button("Browse...").clicked()
            && let Some(file) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_file()
        {
            *value = file.to_string_lossy().into_owned();
        }
    });
}

fn rulers(ui: &mut Ui, s: &mut Settings) {
    let parse = |text: &str| text.split(',').filter_map(|part| part.trim().parse().ok()).collect::<Vec<usize>>();
    let id = Id::new("settings_rulers");
    let kept = ui.data_mut(|d| d.get_temp::<String>(id)).filter(|text| parse(text) == s.rulers);
    let mut text = kept.unwrap_or_else(|| s.rulers.iter().map(usize::to_string).collect::<Vec<_>>().join(", "));
    if field(ui, &mut text, 200.0, "80, 120").changed() {
        s.rulers = parse(&text);
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

fn choose(
    ui: &mut Ui,
    id: &str,
    current: &mut String,
    fixed: &[(&str, &str)],
    imported: &[String],
    confirm: &mut Option<String>,
    remove: fn(&Path, &str),
) -> bool {
    let shown = fixed.iter().find(|(value, _)| value == current).map_or_else(|| current.clone(), |(_, label)| (*label).to_owned());
    let mut removed = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id).width(240.0).selected_text(shown).show_ui(ui, |ui| {
            for (value, label) in fixed {
                ui.selectable_value(current, (*value).to_owned(), *label);
            }
            for name in imported {
                ui.selectable_value(current, name.clone(), name);
            }
        });
        if imported.contains(current) {
            let key = format!("{id}:{current}");
            let armed = confirm.as_deref() == Some(key.as_str());
            if ui.button(if armed { "Click again to remove" } else { "Remove" }).clicked() {
                if armed {
                    remove(&crate::settings::dir(), current);
                    *current = fixed[0].0.to_owned();
                    *confirm = None;
                    removed = true;
                } else {
                    *confirm = Some(key);
                }
            }
        }
    });
    removed
}

fn colors(ui: &mut Ui, theme: &Theme, s: &mut Settings) {
    let mut custom = s.colors.get(&s.theme).cloned().unwrap_or_default();
    let list: Vec<(&str, Color32)> = theme.clone().colors().into_iter().map(|(name, color)| (name, *color)).collect();
    grid(ui, "settings_colors", 7, |ui| {
        for row in 0..10 {
            for at in [row, row + 10] {
                let Some((name, mut color)) = list.get(at).copied() else { continue };
                if ui.color_edit_button_srgba(&mut color).changed() {
                    custom.insert(name.to_owned(), color.to_hex());
                }
                ui.label(RichText::new(name).color(theme.text));
                if !custom.contains_key(name) {
                    ui.allocate_space(vec2(24.0, 24.0));
                } else if ui::icon_button(ui, theme, icons::ARROW_COUNTER_CLOCKWISE, 24.0, false, "Reset").clicked() {
                    custom.remove(name);
                }
                ui.allocate_space(vec2(24.0, 1.0));
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    if ui.add_enabled(!custom.is_empty(), egui::Button::new("Reset All Colors")).clicked() {
        custom.clear();
    }
    if custom.is_empty() {
        s.colors.remove(&s.theme);
    } else {
        s.colors.insert(s.theme.clone(), custom);
    }
}

fn args_edit(ui: &mut Ui, id: Id, args: &mut Vec<String>, width: f32) {
    let kept = ui.data_mut(|d| d.get_temp::<String>(id)).filter(|text| text.split_whitespace().eq(args.iter().map(String::as_str)));
    let mut text = kept.unwrap_or_else(|| args.join(" "));
    if field(ui, &mut text, width, "arguments").changed() {
        *args = text.split_whitespace().map(str::to_owned).collect();
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

fn profiles(ui: &mut Ui, theme: &Theme, s: &mut Settings) {
    let mut remove = None;
    grid(ui, "settings_profiles", 4, |ui| {
        for (index, profile) in s.terminals.iter_mut().enumerate() {
            field(ui, &mut profile.name, 130.0, "name");
            field(ui, &mut profile.program, 250.0, "program");
            args_edit(ui, Id::new(("settings_profile_args", index)), &mut profile.args, 140.0);
            if remove_button(ui, theme) {
                remove = Some(index);
            }
            ui.end_row();
        }
    });
    if let Some(index) = remove {
        s.terminals.remove(index);
    }
    ui.add_space(6.0);
    if ui.button("Add Profile").clicked() {
        s.terminals.push(ShellProfile { name: String::new(), program: String::new(), args: vec![] });
    }
}

fn map_editor(ui: &mut Ui, theme: &Theme, id: &str, map: &mut BTreeMap<String, String>, new: &mut (String, String), hint: &str) {
    let mut remove = None;
    grid(ui, id, 3, |ui| {
        for (key, value) in map.iter_mut() {
            ui.label(RichText::new(key).color(theme.text));
            field(ui, value, 380.0, hint);
            if remove_button(ui, theme) {
                remove = Some(key.clone());
            }
            ui.end_row();
        }
        let patterns = id == "settings_nesting";
        field(ui, &mut new.0, 90.0, if patterns { "*.ts" } else { "extension" });
        field(ui, &mut new.1, 380.0, hint);
        let key = if patterns { new.0.trim().to_owned() } else { new.0.trim().trim_start_matches('.').to_lowercase() };
        if ui.add_enabled(!key.is_empty() && !new.1.trim().is_empty(), egui::Button::new("Add")).clicked() {
            map.insert(key, new.1.trim().to_owned());
            *new = Default::default();
        }
        ui.end_row();
    });
    if let Some(key) = remove {
        map.remove(&key);
    }
}

fn exclude_editor(ui: &mut Ui, theme: &Theme, patterns: &mut Vec<String>, new: &mut String) {
    let mut remove = None;
    grid(ui, "settings_exclude", 2, |ui| {
        for (index, pattern) in patterns.iter_mut().enumerate() {
            field(ui, pattern, 380.0, "pattern");
            if remove_button(ui, theme) {
                remove = Some(index);
            }
            ui.end_row();
        }
        field(ui, new, 380.0, "**/node_modules");
        if ui.add_enabled(!new.trim().is_empty(), egui::Button::new("Add")).clicked() {
            patterns.push(new.trim().to_owned());
            new.clear();
        }
        ui.end_row();
    });
    if let Some(index) = remove {
        patterns.remove(index);
    }
}

fn tasks(ui: &mut Ui, theme: &Theme, s: &mut Settings) {
    let mut remove = None;
    grid(ui, "settings_tasks", 3, |ui| {
        for (index, task) in s.tasks.iter_mut().enumerate() {
            field(ui, &mut task.name, 130.0, "name");
            field(ui, &mut task.command, 340.0, "command");
            if remove_button(ui, theme) {
                remove = Some(index);
            }
            ui.end_row();
        }
    });
    if let Some(index) = remove {
        s.tasks.remove(index);
    }
    ui.add_space(6.0);
    if ui.button("Add Task").clicked() {
        s.tasks.push(Task { name: String::new(), command: String::new() });
    }
}

fn grammars(ui: &mut Ui, s: &mut Settings) {
    ui.horizontal(|ui| {
        field(ui, &mut s.grammars, 320.0, "folder");
        if ui.button("Browse...").clicked()
            && let Some(folder) = rfd::FileDialog::new().pick_folder()
        {
            s.grammars = folder.to_string_lossy().into_owned();
        }
        if ui.button("Add Grammar...").clicked()
            && let Some(file) = rfd::FileDialog::new().add_filter("Grammars", &["sublime-syntax"]).pick_file()
        {
            let target = Path::new(&s.grammars).join(file.file_name().unwrap_or_default());
            let copied = std::fs::create_dir_all(&s.grammars).and_then(|_| std::fs::copy(&file, &target));
            if let Err(error) = copied {
                ui::notify(format!("Could not add {}: {error}", file.display()));
            }
        }
    });
}

fn installed(ui: &mut Ui, theme: &Theme) {
    let languages = crate::editor::languages();
    ui.add_space(20.0);
    ui.label(RichText::new("Languages: Installed").size(13.5).color(theme.text));
    ui.label(RichText::new(format!("{} languages are installed.", languages.len())).color(theme.text_dim));
    ui.add_space(5.0);
    for (name, extensions) in languages {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
        let names = ui.painter().with_clip_rect(rect.with_max_x(rect.left() + 218.0));
        names.text(rect.left_center(), Align2::LEFT_CENTER, name, FontId::proportional(13.0), theme.text);
        let at = rect.left_center() + vec2(230.0, 0.0);
        ui.painter().with_clip_rect(rect).text(at, Align2::LEFT_CENTER, extensions, FontId::proportional(12.5), theme.text_dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(text: &str) -> Vec<&'static str> {
        let query = parse_query(text);
        let mut found: Vec<(&'static str, u32)> = SETTINGS.iter().filter_map(|m| score(m, &query.words).map(|s| (m.key, s))).collect();
        found.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
        found.into_iter().map(|(key, _)| key).collect()
    }

    #[test]
    fn search_ranks_titles_first_matches_keys_fuzzily_and_reads_filters() {
        let query = parse_query("@modified Tab @lang:Rust size");
        assert!(query.modified);
        assert_eq!(query.lang.as_deref(), Some("rust"));
        assert_eq!(query.words, ["tab", "size"]);
        assert_eq!(parse_query("@lang:").lang, None);
        assert_eq!(words("editor.tabSize trim_trailing"), ["editor", "tab", "size", "trim", "trailing"]);
        assert_eq!(rank("tab size")[0], "tab_size");
        assert_eq!(rank("editor.tabSize")[0], "tab_size");
        assert_eq!(rank("files.autoSave")[0], "auto_save");
        assert_eq!(rank("render whitespace")[0], "render_whitespace");
        assert_eq!(rank("wrd wrp")[0], "word_wrap");
        assert!(rank("whitespace").starts_with(&["render_whitespace", "trim_trailing_whitespace"]));
        assert!(rank("font").iter().take(2).all(|k| k.starts_with("font")));
        assert!(rank("zzzz").is_empty());
        assert_eq!(rank("").len(), SETTINGS.len());
        let job = highlight("Tab Size", &["size".to_owned()], Color32::WHITE, Color32::RED, 13.0);
        assert_eq!(job.sections.len(), 2);
        assert_eq!(job.sections[1].format.color, Color32::RED);
    }
}
