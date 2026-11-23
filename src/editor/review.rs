use super::diff::{Hunk, Row};
use super::diff_view::{DiffTab, GAP};
use super::{Editors, notice};
use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, Area, Id, Layout, Rect, RichText, TextEdit, Ui, UiBuilder, pos2, vec2};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

const QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){\
id baseRefOid headRefOid baseRefName files(first:100){nodes{path}} reviewThreads(first:100){nodes{id isResolved isOutdated path line \
originalLine diffSide comments(first:100){nodes{author{login} body createdAt}}}}}}}";
const COMMENT: &str = "mutation($pr:ID!,$path:String!,$line:Int!,$side:DiffSide!,$body:String!){addPullRequestReview(input:{\
pullRequestId:$pr,event:COMMENT,threads:[{path:$path,line:$line,side:$side,body:$body}]}){pullRequestReview{id}}}";
const REPLY: &str = "mutation($id:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$id,body:$body}){comment{id}}}";
const RESOLVE: &str = "mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{id}}}";
const UNRESOLVE: &str = "mutation($id:ID!){unresolveReviewThread(input:{threadId:$id}){thread{id}}}";

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Login {
    login: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct File {
    path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Comment {
    author: Option<Login>,
    body: String,
    created_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Thread {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    path: String,
    line: Option<usize>,
    original_line: Option<usize>,
    diff_side: String,
    comments: Nodes<Comment>,
}

impl Thread {
    fn anchor(&self) -> Option<(usize, bool)> {
        Some((self.line.or(self.original_line)?.checked_sub(1)?, self.diff_side == "LEFT"))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Pr {
    id: String,
    base_ref_oid: String,
    head_ref_oid: String,
    base_ref_name: String,
    files: Nodes<File>,
    review_threads: Nodes<Thread>,
}

#[derive(Clone, Debug, PartialEq)]
enum Change {
    Comment { path: String, line: usize, left: bool, body: String },
    Reply(String, String),
    Resolve(String, bool),
}

#[derive(Clone, Debug, PartialEq)]
struct Draft {
    path: String,
    line: usize,
    left: bool,
    text: String,
}

enum Msg {
    Opened(u64, Result<(Pr, String), String>),
    Updated(u64, Result<Pr, String>),
}

pub(super) struct Review {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    top: PathBuf,
    number: u64,
    inline: bool,
    pr: Option<Pr>,
    base: String,
    open: HashMap<String, bool>,
    replies: HashMap<String, String>,
    draft: Option<Draft>,
    sending: Option<Change>,
    focus: bool,
}

impl Default for Review {
    fn default() -> Self {
        let (tx, rx) = channel();
        let (open, replies) = (HashMap::new(), HashMap::new());
        Self { tx, rx, top: PathBuf::new(), number: 0, inline: false, pr: None, base: String::new(), open, replies, draft: None, sending: None, focus: false }
    }
}

fn graphql(query: &str, fields: &[(&str, &str, String)]) -> Vec<String> {
    let mut args = vec!["api".to_owned(), "graphql".to_owned(), "-f".to_owned(), format!("query={query}")];
    for (flag, name, value) in fields {
        args.extend([flag.to_string(), format!("{name}={value}")]);
    }
    args
}

fn load_args(number: u64) -> Vec<String> {
    graphql(QUERY, &[("-F", "owner", "{owner}".into()), ("-F", "name", "{repo}".into()), ("-F", "number", number.to_string())])
}

fn change_args(pr: &str, change: &Change) -> Vec<String> {
    match change {
        Change::Comment { path, line, left, body } => {
            let side = if *left { "LEFT" } else { "RIGHT" };
            let fields = [("-f", "pr", pr.to_owned()), ("-f", "path", path.clone()), ("-F", "line", (line + 1).to_string()), ("-f", "side", side.into()), ("-f", "body", body.clone())];
            graphql(COMMENT, &fields)
        }
        Change::Reply(id, body) => graphql(REPLY, &[("-f", "id", id.clone()), ("-f", "body", body.clone())]),
        Change::Resolve(id, resolve) => graphql(if *resolve { RESOLVE } else { UNRESOLVE }, &[("-f", "id", id.clone())]),
    }
}

fn parse(json: &str) -> Result<Pr, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("Could not read the pull request: {e}"))?;
    if let Some(error) = value["errors"][0]["message"].as_str() {
        return Err(error.to_owned());
    }
    serde_json::from_value(value["data"]["repository"]["pullRequest"].clone()).map_err(|e| format!("Could not read the pull request: {e}"))
}

fn gh(top: &Path, args: &[String]) -> Result<String, String> {
    crate::git::command("gh", top, &args.iter().map(String::as_str).collect::<Vec<_>>())
}

fn load(top: &Path, number: u64) -> Result<Pr, String> {
    parse(&gh(top, &load_args(number))?)
}

fn fetch(top: &Path, number: u64) -> Result<(Pr, String), String> {
    let pr = load(top, number)?;
    let have = |oid: &str| crate::git::exec(top, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok();
    if !have(&pr.head_ref_oid) || !have(&pr.base_ref_oid) {
        crate::git::exec(top, &["fetch", "--no-tags", "origin", &format!("refs/pull/{number}/head"), &pr.base_ref_name])?;
    }
    let base = crate::git::exec(top, &["merge-base", &pr.base_ref_oid, &pr.head_ref_oid])?.trim().to_owned();
    Ok((pr, base))
}

fn row_of(hunks: &[Hunk], starts: &[u32], pads: &[(usize, u32)], line: usize, left: bool) -> Option<usize> {
    let start = |l: usize| starts.get(l).map(|r| *r as usize);
    if !left {
        return start(line).filter(|_| line + 1 < starts.len());
    }
    let mut shift = 0isize;
    for h in hunks {
        if h.left.contains(&line) {
            let pad = pads.iter().find(|p| p.0 == h.right.start).map_or(0, |p| p.1 as usize);
            return Some(start(h.right.start)?.saturating_sub(pad) + line - h.left.start);
        }
        if h.left.start > line {
            break;
        }
        shift += h.right.len() as isize - h.left.len() as isize;
    }
    let right = (line as isize + shift).max(0) as usize;
    start(right).filter(|_| right + 1 < starts.len())
}

fn line_at(starts: &[u32], aligned: &[Row], row: usize, left: bool) -> Option<usize> {
    if left {
        return aligned.get(row)?.left;
    }
    let line = starts.partition_point(|s| (*s as usize) <= row).checked_sub(1)?;
    (line + 1 < starts.len() && starts[line] as usize == row).then_some(line)
}

impl Review {
    fn submit(&mut self, change: Change, ctx: &egui::Context) {
        let Some(pr) = &self.pr else { return };
        let args = change_args(&pr.id, &change);
        self.sending = Some(change);
        let (tx, ctx, top, number) = (self.tx.clone(), ctx.clone(), self.top.clone(), self.number);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Updated(number, gh(&top, &args).and_then(|_| load(&top, number))));
            ctx.request_repaint();
        });
    }

    fn of(&self, d: &DiffTab) -> Option<&Pr> {
        let (_, left, right, ..) = d.parts();
        self.pr.as_ref().filter(|pr| left == self.base && right == Some(pr.head_ref_oid.as_str()))
    }
}

impl Editors {
    pub fn review_pr(&mut self, top: PathBuf, number: u64, inline: bool, ctx: &egui::Context) {
        let review = std::mem::take(&mut self.review);
        self.review = Review { tx: review.tx, rx: review.rx, top: top.clone(), number, inline, ..Review::default() };
        let (tx, ctx) = (self.review.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = crate::git::tracked(Some(format!("Loading pull request #{number}")), |_| fetch(&top, number));
            let _ = tx.send(Msg::Opened(number, result));
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_review(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.review.rx.try_recv() {
            let review = &mut self.review;
            match msg {
                Msg::Opened(number, Ok((pr, base))) if number == review.number => {
                    let (top, inline) = (review.top.clone(), review.inline);
                    let files: Vec<String> = pr.files.nodes.iter().map(|f| f.path.clone()).collect();
                    let head = pr.head_ref_oid.clone();
                    (review.pr, review.base) = (Some(pr), base.clone());
                    if files.is_empty() {
                        ui::notify_info(format!("Pull request #{number} changes no files."));
                    }
                    let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
                    for path in &files {
                        let name = match files.iter().filter(|f| name(f) == name(path)).count() {
                            1 => name(path),
                            _ => path.clone(),
                        };
                        let args = ["diff", &base, &head, "--", path].map(str::to_owned).to_vec();
                        self.compare(&format!("{name} (#{number})"), &args, &top, inline, ctx);
                    }
                }
                Msg::Updated(number, result) if number == review.number => match result {
                    Ok(pr) => {
                        match review.sending.take() {
                            Some(Change::Comment { .. }) => review.draft = None,
                            Some(Change::Reply(id, _)) => {
                                review.replies.remove(&id);
                            }
                            _ => {}
                        }
                        review.pr = Some(pr);
                    }
                    Err(error) => {
                        review.sending = None;
                        ui::notify_error(error);
                    }
                },
                _ => {}
            }
        }
    }

    pub(super) fn review_exec(&mut self, cmd: &Command) -> bool {
        if *cmd != Command::AddReviewComment {
            return false;
        }
        let found = self.active_tab().and_then(|(g, t)| {
            let tab = &self.groups[g].tabs[t];
            let (d, doc) = (tab.diff.as_deref()?, self.docs.get(&tab.doc)?);
            self.review.of(d)?;
            Some((d.parts().0.to_owned(), doc.buf.line_of(tab.sels.first().map_or(0, |s| s.head))))
        });
        match found {
            Some((path, line)) => {
                self.review.draft = Some(Draft { path, line, left: false, text: String::new() });
                self.review.focus = true;
            }
            None => self.banner = notice("Open a file of the pull request under review first.".to_owned()),
        }
        true
    }

    pub(super) fn review_ui(&mut self, ui: &mut Ui, area: Rect, theme: &Theme, d: &DiffTab, starts: &[u32], pads: &[(usize, u32)]) {
        let review = &mut self.review;
        let (rel, left_rev, right_rev, hunks, aligned) = d.parts();
        let Some(pr) = review.pr.as_ref().filter(|pr| left_rev == review.base && right_rev == Some(pr.head_ref_oid.as_str())) else { return };
        let column = |left: bool| match left && !d.inline {
            true => area.with_max_x(area.center().x - GAP / 2.0),
            false if d.inline => area,
            false => area.with_min_x(area.center().x + GAP / 2.0),
        };
        let y_of = |row: usize| d.text_top + row as f32 * d.line_h - d.scroll;
        let visible = |y: f32| y + d.line_h > area.top() && y < area.bottom();
        let threads: Vec<(&Thread, f32, bool)> = pr.review_threads.nodes.iter()
            .filter(|t| t.path == rel)
            .filter_map(|t| {
                let (line, left) = t.anchor()?;
                Some((t, y_of(row_of(hunks, starts, pads, line, left)?), left))
            })
            .collect();
        let mut changed = None;
        let size = d.line_h.max(16.0);
        for (thread, y, left) in threads.iter().filter(|t| visible(t.1)) {
            let at = Rect::from_min_size(pos2(column(*left).left() + 2.0, *y + (d.line_h - size) / 2.0), vec2(size, size));
            let open = review.open.get(&thread.id).copied().unwrap_or(!thread.is_resolved);
            let count = thread.comments.nodes.len();
            let tip = format!("{count} comment{}{}", if count == 1 { "" } else { "s" }, if thread.is_resolved { ", resolved" } else { "" });
            let icon = if thread.is_resolved { icons::CHECK_CIRCLE } else { icons::CHAT_CIRCLE_TEXT };
            if ui.scope_builder(UiBuilder::new().max_rect(at), |ui| ui::icon_button(ui, theme, icon, size, open, &tip)).inner.clicked() {
                review.open.insert(thread.id.clone(), !open);
            }
        }
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()).filter(|_| review.sending.is_none()) {
            let left = !d.inline && pos.x < area.center().x;
            let row = ((pos.y - d.text_top + d.scroll) / d.line_h).floor();
            let line = (column(left).contains(pos) && row >= 0.0).then(|| line_at(starts, aligned, row as usize, left)).flatten();
            let taken = |line: usize| threads.iter().any(|t| t.0.anchor() == Some((line, left)));
            if let Some(line) = line.filter(|l| !taken(*l)) {
                let y = y_of(row as usize);
                let at = Rect::from_min_size(pos2(column(left).left() + 2.0, y + (d.line_h - size) / 2.0), vec2(size, size));
                if ui.scope_builder(UiBuilder::new().max_rect(at), |ui| ui::icon_button(ui, theme, icons::PLUS, size, false, "Add a comment")).inner.clicked() {
                    review.draft = Some(Draft { path: rel.to_owned(), line, left, text: String::new() });
                    review.focus = true;
                }
            }
        }
        let mut cards: Vec<(f32, bool, Option<&Thread>)> = threads.iter()
            .filter(|t| review.open.get(&t.0.id).copied().unwrap_or(!t.0.is_resolved))
            .map(|t| (t.1 + d.line_h, t.2, Some(t.0)))
            .collect();
        if let Some(draft) = review.draft.as_ref().filter(|draft| draft.path == rel) {
            let row = row_of(hunks, starts, pads, draft.line, draft.left);
            cards.extend(row.map(|row| (y_of(row) + d.line_h, draft.left, None)));
        }
        cards.sort_by(|a, b| a.0.total_cmp(&b.0));
        let busy = review.sending.is_some();
        let mut floor = area.top();
        for (y, left, thread) in cards {
            if !visible(y - d.line_h) {
                continue;
            }
            let col = column(left);
            let width = (col.width() - 48.0).clamp(160.0, 560.0);
            let key = thread.map_or("draft", |t| t.id.as_str());
            let shown = Area::new(Id::new(("pr_thread", key))).fixed_pos(pos2(col.left() + 24.0, y.max(floor))).constrain_to(area).show(ui.ctx(), |ui| {
                super::intel::popup_frame(theme).show(ui, |ui| {
                    ui.set_width(width);
                    match thread {
                        Some(thread) => {
                            let line = thread.anchor().map_or(0, |a| a.0) + 1;
                            ui.horizontal(|ui| {
                                let outdated = if thread.is_outdated { ", outdated" } else { "" };
                                let resolved = if thread.is_resolved { ", resolved" } else { "" };
                                ui.label(RichText::new(format!("Line {line}{outdated}{resolved}")).size(11.5).color(theme.text_dim));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui::icon_button(ui, theme, icons::X, 20.0, false, "Hide").clicked() {
                                        review.open.insert(thread.id.clone(), false);
                                    }
                                    let label = if thread.is_resolved { "Unresolve" } else { "Resolve" };
                                    if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                                        changed = Some(Change::Resolve(thread.id.clone(), !thread.is_resolved));
                                    }
                                    if busy {
                                        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                                        ui::spinner(ui, r.center(), 6.0, theme.accent);
                                    }
                                });
                            });
                            for comment in &thread.comments.nodes {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(comment.author.as_ref().map_or("ghost", |a| a.login.as_str())).strong().color(theme.text));
                                    ui.label(RichText::new(comment.created_at.get(..10).unwrap_or(&comment.created_at)).size(11.0).color(theme.text_dim));
                                });
                                ui.label(RichText::new(&comment.body).color(theme.text));
                            }
                            ui.add_space(6.0);
                            let reply = review.replies.entry(thread.id.clone()).or_default();
                            ui.add(TextEdit::multiline(reply).hint_text("Reply...").desired_rows(2).desired_width(f32::INFINITY));
                            if ui.add_enabled(!busy && !reply.trim().is_empty(), egui::Button::new("Reply")).clicked() {
                                changed = Some(Change::Reply(thread.id.clone(), reply.trim().to_owned()));
                            }
                        }
                        None => {
                            let Some(draft) = review.draft.as_mut() else { return };
                            let side = if draft.left { " (original)" } else { "" };
                            ui.label(RichText::new(format!("New comment on line {}{side}", draft.line + 1)).size(11.5).color(theme.text_dim));
                            let edit = ui.add(TextEdit::multiline(&mut draft.text).hint_text("Comment...").desired_rows(3).desired_width(f32::INFINITY));
                            if std::mem::take(&mut review.focus) {
                                edit.request_focus();
                            }
                            let mut cancel = false;
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!busy && !draft.text.trim().is_empty(), egui::Button::new("Comment")).clicked() {
                                    let (path, line, left, body) = (draft.path.clone(), draft.line, draft.left, draft.text.trim().to_owned());
                                    changed = Some(Change::Comment { path, line, left, body });
                                }
                                cancel = ui.button("Cancel").clicked();
                                if busy {
                                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                                    ui::spinner(ui, r.center(), 6.0, theme.accent);
                                }
                            });
                            if cancel {
                                review.draft = None;
                            }
                        }
                    }
                });
            });
            floor = shown.response.rect.bottom() + 4.0;
        }
        if let Some(change) = changed {
            review.submit(change, ui.ctx());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::time::Duration;

    const FIXTURE: &str = r#"{"data":{"repository":{"pullRequest":{"id":"PR_1","baseRefOid":"b0","headRefOid":"h1","baseRefName":"trunk",
        "files":{"nodes":[{"path":"src/a.rs"},{"path":"b.txt"}]},
        "reviewThreads":{"nodes":[
            {"id":"T1","isResolved":false,"isOutdated":false,"path":"src/a.rs","line":3,"originalLine":3,"diffSide":"RIGHT",
             "comments":{"nodes":[{"author":{"login":"ana"},"body":"Why?","createdAt":"2026-01-02T03:04:05Z"},{"author":null,"body":"Because.","createdAt":"2026-01-03T00:00:00Z"}]}},
            {"id":"T2","isResolved":true,"isOutdated":true,"path":"b.txt","line":null,"originalLine":2,"diffSide":"LEFT","comments":{"nodes":[]}}
        ]}}}}}"#;

    #[test]
    fn reads_review_threads_from_graphql() {
        let pr = parse(FIXTURE).unwrap();
        assert_eq!((pr.id.as_str(), pr.base_ref_oid.as_str(), pr.head_ref_oid.as_str(), pr.base_ref_name.as_str()), ("PR_1", "b0", "h1", "trunk"));
        assert_eq!(pr.files.nodes.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["src/a.rs", "b.txt"]);
        let [first, second] = &pr.review_threads.nodes[..] else { panic!() };
        assert_eq!((first.anchor(), first.comments.nodes.len(), first.comments.nodes[1].author.as_ref()), (Some((2, false)), 2, None));
        assert_eq!(first.comments.nodes[0].author.as_ref().map(|a| a.login.as_str()), Some("ana"));
        assert_eq!((second.anchor(), second.is_resolved, second.is_outdated), (Some((1, true)), true, true));
        assert_eq!(parse(r#"{"errors":[{"message":"Could not resolve to a PullRequest"}]}"#), Err("Could not resolve to a PullRequest".to_owned()));
        assert!(parse(r#"{"data":{"repository":{"pullRequest":null}}}"#).is_err());
    }

    #[test]
    fn builds_gh_graphql_requests() {
        let args = load_args(42);
        assert_eq!(&args[..3], ["api", "graphql", "-f"]);
        assert!(args[3].starts_with("query=query($owner:String!") && args[3].contains("reviewThreads(first:100)"));
        assert_eq!(&args[4..], ["-F", "owner={owner}", "-F", "name={repo}", "-F", "number=42"]);
        let comment = change_args("PR_1", &Change::Comment { path: "src/a.rs".into(), line: 4, left: true, body: "@x hi\nthere".into() });
        assert!(comment[3].contains("addPullRequestReview(") && comment[3].contains("event:COMMENT"));
        assert_eq!(&comment[4..], ["-f", "pr=PR_1", "-f", "path=src/a.rs", "-F", "line=5", "-f", "side=LEFT", "-f", "body=@x hi\nthere"]);
        let reply = change_args("PR_1", &Change::Reply("T1".into(), "ok".into()));
        assert!(reply[3].contains("addPullRequestReviewThreadReply"));
        assert_eq!(&reply[4..], ["-f", "id=T1", "-f", "body=ok"]);
        assert!(change_args("PR_1", &Change::Resolve("T1".into(), true))[3].contains("resolveReviewThread(input:{threadId:$id})"));
        assert!(change_args("PR_1", &Change::Resolve("T1".into(), false))[3].contains("unresolveReviewThread"));
    }

    #[test]
    fn places_threads_on_their_diff_rows() {
        let hunks = [Hunk { left: 2..4, right: 2..3 }, Hunk { left: 6..6, right: 5..7 }];
        let inline_pads = super::super::diff::inline_pads(&hunks);
        let mut starts = vec![];
        let mut at = 0;
        for line in 0..10u32 {
            at += inline_pads.iter().find(|p| p.0 == line as usize).map_or(0, |p| p.1);
            starts.push(at + line);
        }
        assert_eq!(row_of(&hunks, &starts, &inline_pads, 0, false), Some(0));
        assert_eq!(row_of(&hunks, &starts, &inline_pads, 2, false), Some(4));
        assert_eq!(row_of(&hunks, &starts, &inline_pads, 3, true), Some(3));
        assert_eq!(row_of(&hunks, &starts, &inline_pads, 5, true), Some(6));
        assert_eq!(row_of(&hunks, &starts, &inline_pads, 7, true), Some(10));
        assert_eq!(row_of(&hunks, &starts, &inline_pads, 9, false), None);
        let side_pads = super::super::diff::pads(&hunks, false);
        let starts: Vec<u32> = (0..10u32).map(|l| l + if l >= 3 { 1 } else { 0 }).collect();
        assert_eq!(side_pads, [(3, 1)]);
        assert_eq!(row_of(&hunks, &starts, &side_pads, 3, true), Some(3));
        assert_eq!(row_of(&hunks, &starts, &side_pads, 7, true), Some(9));
        assert_eq!((line_at(&starts, &[], 4, false), line_at(&starts, &[], 3, false)), (Some(3), None));
    }

    #[test]
    fn shows_threads_inline_in_the_review_diff_and_starts_a_comment_at_the_caret() {
        let dir = std::env::temp_dir().join(format!("halo-review-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let git = |args: &[&str]| crate::git::exec(&dir, args).unwrap().trim().to_owned();
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"]] {
            git(args);
        }
        std::fs::write(dir.join("src/a.rs"), "one\ntwo\nthree\nfour\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        std::fs::write(dir.join("src/a.rs"), "one\ntwo\nTHREE\nfour\nfive\n").unwrap();
        git(&["commit", "-q", "-am", "head"]);
        let (base, head) = (git(&["rev-parse", "HEAD^"]), git(&["rev-parse", "HEAD"]));
        let mut pr = parse(FIXTURE).unwrap();
        pr.head_ref_oid = head.clone();
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.review = Review { top: dir.clone(), number: 7, pr: Some(pr), base: base.clone(), ..Review::default() };
        editors.compare("a.rs (#7)", &["diff", &base, &head, "--", "src/a.rs"].map(str::to_owned), &dir, true, &ctx);
        let (theme, repo, settings) = (Theme::dark(), crate::git::Git::new(), Settings::default());
        let frame = |editors: &mut Editors| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))), focused: true, ..Default::default() };
            ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &repo, &mut vec![])).textures_delta.clear();
        };
        for _ in 0..500 {
            frame(&mut editors);
            if editors.active_diff_hunks().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        frame(&mut editors);
        frame(&mut editors);
        assert!(ctx.memory(|m| m.area_rect(Id::new(("pr_thread", "T1")))).is_some());
        assert!(ctx.memory(|m| m.area_rect(Id::new(("pr_thread", "T2")))).is_none());
        assert!(editors.exec(&Command::AddReviewComment, &settings, &ctx));
        assert_eq!(editors.review.draft.as_ref().map(|d| (d.path.as_str(), d.line, d.left)), Some(("src/a.rs", 2, false)));
        frame(&mut editors);
        frame(&mut editors);
        assert!(ctx.memory(|m| m.area_rect(Id::new(("pr_thread", "draft")))).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
