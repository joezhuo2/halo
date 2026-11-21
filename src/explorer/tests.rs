use super::*;

fn entry(parent: &Path, name: &str, is_dir: bool) -> Entry {
    Entry { path: parent.join(name), name: name.into(), is_dir }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("halo-explorer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(explorer: &Explorer) -> Vec<&str> {
    explorer.rows.iter().map(|r| r.name.as_str()).collect()
}

#[test]
fn closing_rows_are_visual_only_and_reverse_in_place() {
    let mut explorer = Explorer::new();
    let root = PathBuf::from("root");
    let dir = root.join("src");
    explorer.root = Some(root.clone());
    explorer.children.insert(root.clone(), vec![entry(&root, "src", true), entry(&root, "README", false)]);
    explorer.children.insert(dir.clone(), vec![entry(&dir, "main.rs", false)]);
    explorer.expand(&dir);
    explorer.build();

    explorer.toggle_folders(Some(dir.clone()), 0.0);
    assert_eq!(explorer.rows.len(), 2);
    let closing = explorer.animation.as_ref().unwrap();
    assert_eq!(closing.outgoing[1].path, dir.join("main.rs"));
    assert_eq!(closing.spans, [(0..1, false), (1..2, true), (2..3, false)]);
    assert_eq!(closing.progress(FOLDER_TIME), 0.0);

    let halfway = FOLDER_TIME / 2.0;
    explorer.toggle_folders(Some(dir.clone()), halfway);
    assert_eq!(explorer.rows.len(), 3);
    assert_eq!(explorer.animation.as_ref().unwrap().progress(halfway), 0.5);
    explorer.sel.set(dir.join("main.rs"));
    explorer.toggle_folders(None, 2.0 * FOLDER_TIME);
    assert_eq!(explorer.sel.cursor, Some(dir.clone()));
    assert_eq!(explorer.sel.paths, [dir]);
    assert_eq!(explorer.rows.len(), 2);
    assert!(explorer.expanded.is_empty());
}

#[test]
fn sorts_loads_lazily_and_keeps_expansion_on_refresh() {
    let dir = temp("tree");
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    for file in ["b.txt", "A.txt", "src/main.rs", "src/deep/x.rs"] {
        std::fs::write(dir.join(file), "").unwrap();
    }
    let mut explorer = Explorer::new();
    explorer.set_roots(vec![dir.clone()], vec![String::new()], None);
    explorer.exclude = vec!["**/.git".into()];
    explorer.build();
    assert_eq!(names(&explorer), ["src", "A.txt", "b.txt"]);
    assert_eq!(explorer.children.len(), 1);

    explorer.reveal_active(&dir.join("src/deep/x.rs"));
    std::fs::write(dir.join("src/new.rs"), "").unwrap();
    explorer.refresh();
    explorer.build();
    assert_eq!(names(&explorer), ["src", "deep", "x.rs", "main.rs", "new.rs", "A.txt", "b.txt"]);

    let mut cmds = vec![];
    explorer.start_edit(EditKind::NewFile, dir.clone());
    explorer.edit.as_mut().unwrap().text = "A.txt".into();
    explorer.commit_edit(&mut cmds);
    assert!(explorer.error.is_some() && cmds.is_empty());
    explorer.edit.as_mut().unwrap().text = "made.txt".into();
    explorer.commit_edit(&mut cmds);
    assert!(dir.join("made.txt").is_file());
    assert_eq!(cmds[0], Command::OpenPath(dir.join("made.txt")));

    assert!(validate("a/b").is_err() && validate("").is_err() && validate("ok.rs").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn excluded_names_are_hidden() {
    let dir = temp("exclude");
    std::fs::create_dir_all(dir.join("keep")).unwrap();
    std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    std::fs::write(dir.join("a.log"), "").unwrap();
    std::fs::write(dir.join("keep/b.log"), "").unwrap();
    std::fs::write(dir.join("keep/c.rs"), "").unwrap();
    let mut explorer = Explorer::new();
    explorer.set_roots(vec![dir.clone()], vec![String::new()], None);
    explorer.exclude = vec!["**/node_modules".into(), "**/*.log".into()];
    explorer.expand(&dir.join("keep"));
    explorer.build();
    assert_eq!(names(&explorer), ["keep", "c.rs"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn single_folder_chains_become_one_row() {
    let dir = temp("compact");
    std::fs::create_dir_all(dir.join("a/b/c")).unwrap();
    std::fs::create_dir_all(dir.join("x/y")).unwrap();
    std::fs::write(dir.join("a/b/c/f.txt"), "").unwrap();
    std::fs::write(dir.join("x/y/g.txt"), "").unwrap();
    std::fs::write(dir.join("x/h.txt"), "").unwrap();
    let mut explorer = Explorer::new();
    explorer.set_roots(vec![dir.clone()], vec![String::new()], None);
    explorer.compact = true;
    explorer.build();
    assert_eq!(names(&explorer), ["a/b/c", "x"]);
    let chain = &explorer.rows[0];
    assert_eq!(chain.path, dir.join("a/b/c"));
    assert_eq!(chain.chain, [dir.join("a"), dir.join("a/b"), dir.join("a/b/c")]);
    assert!(explorer.rows[1].chain.is_empty());
    assert_eq!(explorer.row_of(&dir.join("a/b")), Some(0));

    explorer.expand(&dir.join("a/b/c"));
    explorer.build();
    assert_eq!(names(&explorer), ["a/b/c", "f.txt", "x"]);
    assert_eq!(explorer.rows[1].depth, 1);

    explorer.start_edit(EditKind::NewFile, dir.join("a/b"));
    explorer.build();
    assert_eq!(names(&explorer), ["a/b", "", "c", "f.txt", "x"]);
    assert!(explorer.rows[1].editing);
    explorer.edit = None;
    explorer.start_edit(EditKind::Rename, dir.join("a/b"));
    explorer.build();
    assert_eq!(names(&explorer)[..3], ["a", "b/c", "f.txt"]);
    assert!(explorer.rows[1].editing);

    explorer.compact = false;
    explorer.edit = None;
    explorer.build();
    assert_eq!(explorer.rows[0].name, "a");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn filtering_hides_rows_or_highlights_them() {
    let dir = temp("filter");
    std::fs::create_dir_all(dir.join("src/ui")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    for file in ["src/ui/mod.rs", "src/lib.rs", "docs/guide.md", "README.md"] {
        std::fs::write(dir.join(file), "").unwrap();
    }
    let mut explorer = Explorer::new();
    explorer.set_roots(vec![dir.clone()], vec![String::new()], None);
    explorer.expand(&dir.join("src"));
    explorer.expand(&dir.join("src/ui"));
    explorer.expand(&dir.join("docs"));
    explorer.build();
    assert_eq!(explorer.rows.len(), 7);

    explorer.filter.open = true;
    explorer.filter.text = "lib".into();
    explorer.dirty = true;
    explorer.build();
    assert_eq!(names(&explorer), ["src", "lib.rs"]);
    assert_eq!(explorer.filter.matches, 1);
    assert_eq!(explorer.rows[1].hits, [0, 1, 2]);

    explorer.filter.hide = false;
    explorer.build();
    assert_eq!(explorer.rows.len(), 7);
    assert_eq!(explorer.rows.iter().filter(|r| !r.hits.is_empty()).count(), 1);

    explorer.filter.hide = true;
    explorer.filter.text = ".md".into();
    explorer.build();
    assert_eq!(names(&explorer), ["docs", "guide.md", "README.md"]);
    explorer.sel.set(dir.join("docs/guide.md"));
    explorer.go_match(true);
    assert_eq!(explorer.sel.cursor, Some(dir.join("README.md")));
    explorer.go_match(true);
    assert_eq!(explorer.sel.cursor, Some(dir.join("docs/guide.md")));
    explorer.go_match(false);
    assert_eq!(explorer.sel.cursor, Some(dir.join("README.md")));
    let _ = std::fs::remove_dir_all(&dir);
}

mod headless {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput};

    fn frame(ctx: &egui::Context, explorer: &mut Explorer, mut events: Vec<Event>, modifiers: Modifiers) -> Vec<Command> {
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 600.0))),
            events,
            focused: true,
            ..Default::default()
        };
        let mut cmds = vec![];
        let settings = Settings { files_exclude: vec![], compact_folders: false, ..Settings::default() };
        let git = Git::new();
        let mut out = ctx.run_ui(input, |ui| explorer.ui(ui, &Theme::dark(), &git, None, &settings, &[], &mut cmds));
        out.textures_delta.clear();
        cmds
    }

    fn click(ctx: &egui::Context, explorer: &mut Explorer, name: &Path, modifiers: Modifiers) -> Vec<Command> {
        let pos = ctx.read_response(Id::new(("explorer_row", name))).unwrap().rect.center();
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
        frame(ctx, explorer, vec![Event::PointerMoved(pos)], modifiers);
        frame(ctx, explorer, vec![button(true)], modifiers);
        frame(ctx, explorer, vec![button(false)], modifiers)
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn setup(name: &str, files: &[&str]) -> (PathBuf, egui::Context, Explorer) {
        let dir = temp(name);
        for file in files {
            std::fs::write(dir.join(file), file).unwrap();
        }
        let ctx = egui::Context::default();
        let mut explorer = Explorer::new();
        explorer.set_roots(vec![dir.clone()], vec![String::new()], None);
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        (dir, ctx, explorer)
    }

    fn wait(ctx: &egui::Context, explorer: &mut Explorer) -> Vec<Command> {
        let mut cmds = vec![];
        for _ in 0..200 {
            cmds.extend(frame(ctx, explorer, vec![], Modifiers::NONE));
            if explorer.work.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cmds
    }

    #[test]
    fn rows_are_tree_items_with_levels_and_expanded_states() {
        use crate::ui::Role;
        let (dir, ctx, mut explorer) = setup("a11y", &["top.txt"]);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        explorer.refresh();
        explorer.expand(&dir.join("src"));
        wait(&ctx, &mut explorer);
        ctx.enable_accesskit();
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 600.0))), ..Default::default() };
        let (settings, git) = (Settings { files_exclude: vec![], compact_folders: false, ..Settings::default() }, Git::new());
        let mut out = ctx.run_ui(input, |ui| explorer.ui(ui, &Theme::dark(), &git, None, &settings, &[], &mut vec![]));
        let nodes = crate::a11y::tree(&mut out);
        let src = crate::a11y::node(&nodes, Role::TreeItem, "src");
        assert_eq!((src.level(), src.is_expanded()), (Some(1), Some(true)));
        let deep = crate::a11y::node(&nodes, Role::TreeItem, "deep");
        assert_eq!((deep.level(), deep.is_expanded()), (Some(2), Some(false)));
        let lib = crate::a11y::node(&nodes, Role::TreeItem, "lib.rs");
        assert_eq!((lib.level(), lib.is_expanded(), lib.is_selected()), (Some(2), None, Some(false)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_select_then_delete_confirms_once_with_the_count() {
        let (dir, ctx, mut explorer) = setup("select", &["a.txt", "b.txt", "c.txt", "d.txt"]);
        click(&ctx, &mut explorer, &dir.join("a.txt"), Modifiers::NONE);
        click(&ctx, &mut explorer, &dir.join("c.txt"), Modifiers::COMMAND);
        assert_eq!(explorer.sel.paths, [dir.join("a.txt"), dir.join("c.txt")]);
        click(&ctx, &mut explorer, &dir.join("d.txt"), Modifiers::SHIFT);
        assert_eq!(explorer.sel.paths, [dir.join("c.txt"), dir.join("d.txt")]);
        click(&ctx, &mut explorer, &dir.join("a.txt"), Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![key(Key::ArrowDown, Modifiers::SHIFT)], Modifiers::SHIFT);
        assert_eq!(explorer.sel.paths, [dir.join("a.txt"), dir.join("b.txt")]);
        frame(&ctx, &mut explorer, vec![key(Key::A, Modifiers::COMMAND)], Modifiers::COMMAND);
        assert_eq!(explorer.sel.paths.len(), 4);
        click(&ctx, &mut explorer, &dir.join("b.txt"), Modifiers::COMMAND);
        click(&ctx, &mut explorer, &dir.join("d.txt"), Modifiers::COMMAND);
        assert_eq!(explorer.sel.paths, [dir.join("a.txt"), dir.join("c.txt")]);

        frame(&ctx, &mut explorer, vec![key(Key::Delete, Modifiers::NONE)], Modifiers::NONE);
        assert_eq!(explorer.confirm_delete, [dir.join("a.txt"), dir.join("c.txt")]);
        assert_eq!(count_text(&explorer.confirm_delete), "2 items");

        let mut cmds = vec![];
        explorer.delete(&mut cmds);
        assert_eq!(cmds.len(), 2);
        assert!(cmds.contains(&Command::PathChanged(dir.join("a.txt"), None)));
        assert!(!dir.join("a.txt").exists() && !dir.join("c.txt").exists());
        assert!(dir.join("b.txt").exists() && dir.join("d.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cut_then_paste_moves_into_the_selected_folder() {
        let (dir, ctx, mut explorer) = setup("paste", &["a.txt", "b.txt"]);
        std::fs::create_dir(dir.join("dest")).unwrap();
        std::fs::write(dir.join("dest/b.txt"), "old").unwrap();
        explorer.refresh();
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);

        click(&ctx, &mut explorer, &dir.join("a.txt"), Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![Event::Cut], Modifiers::NONE);
        assert_eq!(explorer.clipboard, Some((vec![dir.join("a.txt")], true)));
        click(&ctx, &mut explorer, &dir.join("dest"), Modifiers::NONE);
        explorer.expand(&dir.join("dest"));
        let mut cmds = frame(&ctx, &mut explorer, vec![Event::Paste("x".into())], Modifiers::NONE);
        cmds.extend(wait(&ctx, &mut explorer));
        assert!(!dir.join("a.txt").exists() && dir.join("dest/a.txt").is_file());
        assert!(cmds.contains(&Command::PathChanged(dir.join("a.txt"), Some(dir.join("dest/a.txt")))));
        assert!(explorer.clipboard.is_none());
        assert_eq!(explorer.sel.cursor, Some(dir.join("dest/a.txt")));

        explorer.clipboard = Some((vec![dir.join("b.txt")], false));
        explorer.sel.set(dir.join("dest/a.txt"));
        let ctx2 = ctx.clone();
        explorer.paste(&ctx2);
        wait(&ctx, &mut explorer);
        assert_eq!(std::fs::read_to_string(dir.join("dest/b copy.txt")).unwrap(), "b.txt");
        assert_eq!(std::fs::read_to_string(dir.join("dest/b.txt")).unwrap(), "old");
        assert!(dir.join("b.txt").exists());

        explorer.clipboard = Some((vec![dir.join("b.txt")], true));
        explorer.sel.set(dir.join("dest"));
        explorer.paste(&ctx2);
        assert!(explorer.clash.is_some() && dir.join("b.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dropping_a_folder_into_itself_is_refused() {
        let (dir, ctx, mut explorer) = setup("drop", &["a.txt"]);
        std::fs::create_dir_all(dir.join("f/inner")).unwrap();
        explorer.confirm_drag = true;
        explorer.request_move(vec![dir.join("f")], dir.join("f/inner"), &ctx);
        assert!(explorer.confirm_drop.is_none());
        explorer.request_move(vec![dir.join("a.txt")], dir.join("f"), &ctx);
        assert_eq!(explorer.confirm_drop, Some((vec![dir.join("a.txt")], dir.join("f"))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_roots_are_top_level_nodes_that_reorder_by_drag_and_are_never_deleted() {
        let (a, b) = (temp("root-a"), temp("root-b"));
        std::fs::write(a.join("one.txt"), "").unwrap();
        std::fs::write(b.join("two.txt"), "").unwrap();
        let ctx = egui::Context::default();
        let mut explorer = Explorer::new();
        explorer.set_roots(vec![a.clone()], vec!["a".into()], None);
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        assert_eq!(names(&explorer), ["one.txt"]);
        explorer.set_roots(vec![a.clone(), b.clone()], vec!["api".into(), "web".into()], Some("Untitled (Workspace)".into()));
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![], Modifiers::NONE);
        assert_eq!(names(&explorer), ["api", "one.txt", "web", "two.txt"]);
        assert_eq!(explorer.dir_for_new(Some(&b.join("two.txt"))), b);

        click(&ctx, &mut explorer, &b, Modifiers::NONE);
        assert_eq!(names(&explorer), ["api", "one.txt", "web"]);
        frame(&ctx, &mut explorer, vec![key(Key::Delete, Modifiers::NONE)], Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![key(Key::F2, Modifiers::NONE)], Modifiers::NONE);
        assert!(explorer.confirm_delete.is_empty() && explorer.edit.is_none());
        click(&ctx, &mut explorer, &a.join("one.txt"), Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![key(Key::ArrowLeft, Modifiers::NONE)], Modifiers::NONE);
        assert_eq!(explorer.sel.cursor, Some(a.clone()));

        let from = ctx.read_response(Id::new(("explorer_row", &b))).unwrap().rect.center();
        let to = ctx.read_response(Id::new(("explorer_row", &a))).unwrap().rect.center();
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(&ctx, &mut explorer, vec![Event::PointerMoved(from)], Modifiers::NONE);
        frame(&ctx, &mut explorer, vec![button(from, true)], Modifiers::NONE);
        let mut cmds = vec![];
        for step in 1..=4 {
            let pos = from + (to - from) * (step as f32 / 4.0);
            cmds.extend(frame(&ctx, &mut explorer, vec![Event::PointerMoved(pos)], Modifiers::NONE));
        }
        cmds.extend(frame(&ctx, &mut explorer, vec![button(to, false)], Modifiers::NONE));
        cmds.extend(frame(&ctx, &mut explorer, vec![], Modifiers::NONE));
        assert_eq!(cmds, [Command::MoveWorkspaceFolder(1, 0)]);
        assert!(a.join("one.txt").exists() && b.join("two.txt").exists());

        explorer.set_roots(vec![b.clone(), a.clone()], vec!["web".into(), "api".into()], Some("Untitled (Workspace)".into()));
        explorer.build();
        assert_eq!(names(&explorer), ["web", "api", "one.txt"]);
        explorer.set_roots(vec![b.clone()], vec!["web".into()], None);
        explorer.build();
        assert_eq!(names(&explorer), ["two.txt"]);
        let _ = (std::fs::remove_dir_all(&a), std::fs::remove_dir_all(&b));
    }
}

#[test]
fn sort_orders_follow_the_setting() {
    let dir = Path::new("root");
    let mut list = vec![entry(dir, "b.rs", false), entry(dir, "Z", true), entry(dir, "a.txt", false), entry(dir, "c.rs", false)];
    let names = |list: &[Entry]| list.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    sort_entries(&mut list, "mixed");
    assert_eq!(names(&list), ["a.txt", "b.rs", "c.rs", "Z"]);
    sort_entries(&mut list, "filesFirst");
    assert_eq!(names(&list), ["a.txt", "b.rs", "c.rs", "Z"]);
    sort_entries(&mut list, "type");
    assert_eq!(names(&list), ["Z", "b.rs", "c.rs", "a.txt"]);
    sort_entries(&mut list, "default");
    assert_eq!(names(&list), ["Z", "a.txt", "b.rs", "c.rs"]);
}
