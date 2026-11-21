use super::{exec, run};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Stash {
    pub reference: String,
    pub hash: String,
    pub message: String,
    pub branch: String,
    pub age: String,
}

#[derive(Clone, Debug)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

#[derive(Clone, Debug)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub bare: bool,
}

#[derive(Default)]
pub struct Details {
    pub head: String,
    pub has_parent: bool,
    pub message: String,
    pub operation: Option<&'static str>,
    pub stashes: Vec<Stash>,
    pub tags: Vec<String>,
    pub remotes: Vec<Remote>,
    pub worktrees: Vec<Worktree>,
}

const STASH_FORMAT: &str = "--format=%gd%x00%H%x00%cr%x00%gs";

fn parse_stashes(text: &str) -> Vec<Stash> {
    text.lines().filter_map(|line| {
        let mut fields = line.splitn(4, '\0');
        let (reference, hash, age, subject) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
        let rest = subject.strip_prefix("WIP on ").or_else(|| subject.strip_prefix("On ")).unwrap_or(subject);
        let (branch, message) = rest.split_once(": ").unwrap_or(("", rest));
        Some(Stash { reference: reference.into(), hash: hash.into(), message: message.into(), branch: branch.into(), age: age.into() })
    }).collect()
}

fn parse_worktrees(top: &Path, text: &str) -> Vec<Worktree> {
    text.split("\0\0").filter_map(|record| {
        let mut lines = record.split('\0');
        let path = crate::wsl::local(top, lines.next()?.strip_prefix("worktree ")?);
        let mut branch = String::from("detached HEAD");
        let mut bare = false;
        for line in lines {
            if let Some(name) = line.strip_prefix("branch refs/heads/") {
                branch = name.to_owned();
            } else if line == "bare" {
                branch = "bare".to_owned();
                bare = true;
            }
        }
        Some(Worktree { path, branch, bare })
    }).collect()
}

fn parse_remotes(text: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once('\t') else { continue };
        let Some((url, kind)) = rest.rsplit_once(' ') else { continue };
        if remotes.last().is_none_or(|r| r.name != name) {
            remotes.push(Remote { name: name.to_owned(), fetch_url: String::new(), push_url: String::new() });
        }
        let remote = remotes.last_mut().unwrap();
        match kind {
            "(fetch)" => remote.fetch_url = url.to_owned(),
            "(push)" if remote.push_url.is_empty() => remote.push_url = url.to_owned(),
            _ => {}
        }
    }
    remotes
}

pub fn operation(top: &Path) -> Result<Option<&'static str>, String> {
    let dir = crate::wsl::local(top, super::read(top, &["rev-parse", "--absolute-git-dir"])?.trim_end());
    Ok(if dir.join("rebase-merge").is_dir() || dir.join("rebase-apply/rebasing").is_file() {
        Some("rebase")
    } else if dir.join("MERGE_HEAD").is_file() {
        Some("merge")
    } else if dir.join("CHERRY_PICK_HEAD").is_file() {
        Some("cherry-pick")
    } else {
        None
    })
}

pub fn load(top: &Path) -> Result<Details, String> {
    let (operation, [head, stashes, tags, remotes, worktrees]) = std::thread::scope(|s| {
        let operation = s.spawn(|| operation(top));
        let log = ["log", "-1", "--format=%H%x00%P%x00%B"];
        let rest = super::exec_all(top, [&log, &["stash", "list", STASH_FORMAT], &["tag", "--list"], &["remote", "-v"], &["worktree", "list", "--porcelain", "-z"]]);
        (operation.join().unwrap_or_else(|_| Err("Git stopped unexpectedly.".into())), rest)
    });
    let head = head.unwrap_or_default();
    let mut fields = head.trim_end_matches(['\r', '\n']).splitn(3, '\0');
    Ok(Details {
        head: fields.next().unwrap_or_default().to_owned(),
        has_parent: fields.next().is_some_and(|s| !s.is_empty()),
        message: fields.next().unwrap_or_default().to_owned(),
        operation: operation?,
        stashes: parse_stashes(&stashes?),
        tags: tags?.lines().map(str::to_owned).collect(),
        remotes: parse_remotes(&remotes?),
        worktrees: parse_worktrees(top, &worktrees?),
    })
}

pub fn amend(top: &Path, message: &str) -> Result<String, String> {
    if message.trim().is_empty() {
        exec(top, &["commit", "--amend", "--no-edit"])
    } else {
        exec(top, &["commit", "--amend", "-m", message])
    }
}

pub fn undo(top: &Path, head: &str, confirmed: bool) -> Result<String, String> {
    if exec(top, &["rev-parse", "HEAD"])?.trim_end() != head {
        return Err("HEAD changed. Review the new commit before undoing it.".into());
    }
    if !confirmed && run(top, &["merge-base", "--is-ancestor", head, "@{upstream}"]).is_some() {
        return Err("This commit is now on the upstream branch. Choose Undo Last Commit again to confirm.".into());
    }
    exec(top, &["reset", "--soft", "HEAD~1"])
}

pub fn create_tag(top: &Path, name: &str, message: &str, target: &str) -> Result<String, String> {
    if message.is_empty() {
        exec(top, &["tag", "--", name, target])
    } else {
        exec(top, &["tag", "-m", message, "--", name, target])
    }
}

pub fn stash_action(top: &Path, verb: &str, reference: &str, hash: &str) -> Result<String, String> {
    if exec(top, &["rev-parse", "--verify", reference])?.trim_end() != hash {
        return Err("The stash list changed. Select the stash again.".into());
    }
    exec(top, &["stash", verb, reference])
}

pub fn clone_name(url: &str) -> Result<&str, String> {
    let tail = url.trim_end_matches(['/', '\\']).rsplit(['/', '\\', ':']).next().unwrap_or_default();
    let name = tail.strip_suffix(".git").unwrap_or(tail);
    match Path::new(name).components().collect::<Vec<_>>().as_slice() {
        [Component::Normal(_)] => Ok(name),
        _ => Err("The URL has no repository folder name.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stashes_worktrees_remotes_and_clone_names() {
        let stashes = parse_stashes("stash@{0}\0abc\x005 minutes ago\0WIP on main: 1234567 fix\nstash@{1}\0def\x002 days ago\0On dev: my: note\n");
        let fields = |s: &Stash| (s.reference.clone(), s.hash.clone(), s.message.clone(), s.branch.clone(), s.age.clone());
        assert_eq!(fields(&stashes[0]), ("stash@{0}".into(), "abc".into(), "1234567 fix".into(), "main".into(), "5 minutes ago".into()));
        assert_eq!(fields(&stashes[1]), ("stash@{1}".into(), "def".into(), "my: note".into(), "dev".into(), "2 days ago".into()));

        let text = "worktree /r\0HEAD 1\0branch refs/heads/main\0\0worktree /w\0HEAD 2\0detached\0\0worktree /b\0bare\0\0";
        let trees: Vec<_> = parse_worktrees(Path::new("/r"), text).into_iter().map(|w| (w.path, w.branch, w.bare)).collect();
        assert_eq!(trees, [("/r".into(), "main".into(), false), ("/w".into(), "detached HEAD".into(), false), ("/b".into(), "bare".into(), true)]);

        let text = "origin\thttps://x/a.git (fetch)\norigin\tssh://x/a.git (push)\nup\t/path with space (fetch)\nup\t/path with space (push)\n";
        let remotes: Vec<_> = parse_remotes(text).into_iter().map(|r| (r.name, r.fetch_url, r.push_url)).collect();
        assert_eq!(remotes, [("origin".into(), "https://x/a.git".into(), "ssh://x/a.git".into()),
            ("up".into(), "/path with space".into(), "/path with space".into())]);

        assert_eq!(clone_name("https://github.com/o/halo.git/"), Ok("halo"));
        assert_eq!(clone_name("git@host:o/repo"), Ok("repo"));
        assert!(clone_name("https://host/..").is_err());
    }

    #[test]
    fn commits_merges_picks_tags_stashes_and_worktrees() {
        let base = std::env::temp_dir().join(format!("halo-git-ops-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, remote) = (base.join("work"), base.join("remote.git"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&remote).unwrap();
        exec(&remote, &["init", "-q", "--bare"]).unwrap();
        let git = |args: &[&str]| exec(&dir, args).unwrap();
        let write = |text: &str| std::fs::write(dir.join("f.txt"), text).unwrap();
        let commit = |text: &str, message: &str| {
            write(text);
            git(&["commit", "-qam", message]);
        };
        let subject = || git(&["log", "-1", "--format=%s"]).trim_end().to_owned();
        git(&["init", "-q", "-b", "trunk"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        write("base\n");
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        let first = load(&dir).unwrap();
        assert!(!first.has_parent && first.message.trim_end() == "init" && first.operation.is_none());

        commit("two\n", "second");
        write("amended\n");
        git(&["add", "."]);
        amend(&dir, "").unwrap();
        assert_eq!(subject(), "second");
        amend(&dir, "renamed").unwrap();
        assert_eq!(subject(), "renamed");

        let details = load(&dir).unwrap();
        assert!(details.has_parent);
        assert!(undo(&dir, &first.head, false).is_err());
        undo(&dir, &details.head, false).unwrap();
        assert_eq!(subject(), "init");
        assert_eq!(git(&["diff", "--cached", "--name-only"]).trim_end(), "f.txt");
        git(&["commit", "-qm", "again"]);
        git(&["push", "-q", "-u", "origin", "trunk"]);
        let head = load(&dir).unwrap().head;
        assert!(undo(&dir, &head, false).unwrap_err().contains("upstream"));
        undo(&dir, &head, true).unwrap();
        git(&["commit", "-qm", "again"]);

        git(&["switch", "-qc", "side"]);
        commit("side\n", "side change");
        let side = git(&["rev-parse", "HEAD"]).trim_end().to_owned();
        git(&["switch", "-q", "trunk"]);
        commit("trunk\n", "trunk change");
        assert!(exec(&dir, &["merge", "--no-edit", "--", "side"]).is_err());
        assert_eq!(load(&dir).unwrap().operation, Some("merge"));
        assert_eq!(operation(&dir), Ok(Some("merge")));
        git(&["merge", "--abort"]);
        assert_eq!(operation(&dir), Ok(None));
        assert_eq!(subject(), "trunk change");

        assert!(exec(&dir, &["cherry-pick", "--", &side]).is_err());
        assert_eq!(operation(&dir), Ok(Some("cherry-pick")));
        git(&["cherry-pick", "--abort"]);
        assert!(exec(&dir, &["rebase", "--", "side"]).is_err());
        assert_eq!(operation(&dir), Ok(Some("rebase")));
        git(&["-c", "core.editor=true", "rebase", "--skip"]);
        assert_eq!(operation(&dir), Ok(None));
        assert_eq!(subject(), "side change");

        git(&["switch", "-qc", "pick", "trunk"]);
        std::fs::write(dir.join("g.txt"), "g\n").unwrap();
        git(&["add", "g.txt"]);
        git(&["commit", "-qm", "add g"]);
        let g = git(&["rev-parse", "HEAD"]).trim_end().to_owned();
        git(&["switch", "-q", "trunk"]);
        exec(&dir, &["cherry-pick", "--", &g]).unwrap();
        assert_eq!(subject(), "add g");

        create_tag(&dir, "v1", "", "HEAD").unwrap();
        create_tag(&dir, "v2", "release two", &side).unwrap();
        assert_eq!(load(&dir).unwrap().tags, ["v1", "v2"]);
        assert_eq!(git(&["cat-file", "-t", "v2"]).trim_end(), "tag");
        assert_eq!(git(&["cat-file", "-t", "v1"]).trim_end(), "commit");
        git(&["tag", "-d", "--", "v1"]);
        assert_eq!(load(&dir).unwrap().tags, ["v2"]);

        write("stashed\n");
        git(&["stash", "push", "-m", "keep this"]);
        let stash = load(&dir).unwrap().stashes.remove(0);
        assert_eq!((stash.reference.as_str(), stash.message.as_str(), stash.branch.as_str()), ("stash@{0}", "keep this", "trunk"));
        assert!(stash_action(&dir, "apply", "stash@{0}", "0000").is_err());
        stash_action(&dir, "pop", &stash.reference, &stash.hash).unwrap();
        assert!(load(&dir).unwrap().stashes.is_empty());
        git(&["checkout", "--", "f.txt"]);

        let tree = base.join("tree");
        git(&["worktree", "add", "-q", "-b", "wt", "--", tree.to_str().unwrap(), "HEAD"]);
        let trees = load(&dir).unwrap().worktrees;
        assert_eq!(trees.iter().map(|w| w.branch.as_str()).collect::<Vec<_>>(), ["trunk", "wt"]);
        assert_eq!(trees[1].path.canonicalize().unwrap(), tree.canonicalize().unwrap());
        git(&["worktree", "remove", "--", tree.to_str().unwrap()]);
        assert_eq!(load(&dir).unwrap().worktrees.len(), 1);

        let remotes = load(&dir).unwrap().remotes;
        assert_eq!(remotes.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["origin"]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
