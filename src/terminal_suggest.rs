use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    History,
    Command,
    Sub,
    Flag,
    Folder,
    File,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: Kind,
    pub replace: usize,
    pub insert: String,
}

const SPECS: &[(&str, &str, &str, &str)] = &[
    ("git", "", "add bisect blame branch checkout cherry-pick clean clone commit config diff fetch grep init log merge mv pull push rebase remote reset restore revert rm show stash status switch tag worktree", "--version --help -C -c --no-pager"),
    ("git", "add", "", "-A --all -p --patch -u --update -n --dry-run -f --force"),
    ("git", "commit", "", "-m --message -a --all --amend --no-edit -s --signoff -v --fixup --no-verify"),
    ("git", "push", "", "-u --set-upstream -f --force --force-with-lease --tags --delete --dry-run --no-verify"),
    ("git", "pull", "", "--rebase --no-rebase --ff-only --autostash"),
    ("git", "fetch", "", "--all --prune --tags"),
    ("git", "checkout", "", "-b -B -f --track --detach"),
    ("git", "switch", "", "-c -C --detach --track"),
    ("git", "branch", "", "-a --all -d -D -m -M -r -v -vv --list --show-current"),
    ("git", "log", "", "--oneline --graph --all --stat -p --patch --author --since --until -n --decorate --follow"),
    ("git", "diff show", "", "--staged --cached --stat --name-only --name-status --word-diff"),
    ("git", "stash", "list show pop apply drop clear push", "-u --include-untracked -m --message"),
    ("git", "rebase", "", "-i --interactive --continue --abort --skip --onto --autosquash"),
    ("git", "reset", "", "--soft --mixed --hard --keep"),
    ("git", "merge", "", "--no-ff --ff-only --squash --abort --continue"),
    ("git", "clone", "", "--depth --branch --recursive --bare"),
    ("git", "status", "", "-s --short -b --branch"),
    ("git", "restore", "", "--staged --source -W --worktree"),
    ("git", "remote", "add remove rename set-url show", "-v"),
    ("git", "tag", "", "-a -d -l -m"),
    ("git", "worktree", "add list remove prune", ""),
    ("cargo", "", "add bench build check clean clippy doc fix fmt init install new publish remove run search test tree update", "--version --help -V -v --list"),
    ("cargo", "build check run test clippy bench doc", "", "--release --all-targets --workspace -p --package --features --all-features --no-default-features --target --bin --example --lib -j --jobs -q --quiet"),
    ("npm", "", "audit ci exec init install i list ls outdated publish run start test uninstall update version", "--version --help -g --global"),
    ("npm", "install i", "", "--save-dev -D --save -S --global -g --force --legacy-peer-deps"),
    ("pnpm", "", "add dlx exec install i list outdated remove run start test update", "--version --help -g --global"),
    ("yarn", "", "add dlx exec install remove run start test upgrade", "--version --help"),
    ("docker", "", "build compose exec images logs network ps pull push rm rmi run start stop system volume", "--version --help"),
    ("docker", "run", "", "-d --detach -i -t -it --rm -p --publish -v --volume -e --env --name --network -w --workdir"),
    ("docker", "build", "", "-t --tag -f --file --no-cache --build-arg --target"),
    ("docker", "ps", "", "-a --all -q --quiet"),
    ("docker", "compose", "build down exec logs ps pull restart up", "-f --file -d --detach --build"),
    ("python python3 py", "", "", "-m -c -V --version -u -i"),
    ("pip pip3", "", "freeze install list show uninstall", "--version --help"),
    ("pip pip3", "install", "", "-r --requirement -U --upgrade -e --editable --user"),
    ("node", "", "", "-e --eval -v --version --inspect -r --require --watch"),
    ("go", "", "build clean env fmt get install mod run test version vet", "--help"),
    ("go", "test", "", "-v -run -race -cover -count -bench"),
    ("kubectl", "", "apply config delete describe exec get logs port-forward", "-n --namespace -o --output -f --filename -A --all-namespaces"),
    ("gh", "", "api auth issue pr release repo run workflow", "--version --help"),
    ("gh", "pr", "checkout create diff list merge status view", "--web"),
    ("ls", "", "", "-l -a -la -h -R -t"),
    ("grep", "", "", "-r -i -n -v -E -l -w"),
    ("rm", "", "", "-r -f -rf -i"),
    ("cp", "", "", "-r -a -v"),
    ("mkdir", "", "", "-p"),
    ("tar", "", "", "-x -c -z -v -f -xzf -czf"),
    ("curl", "", "", "-X -H -d -o -O -L -s -v -I"),
    ("ssh", "", "", "-p -i -L -R -v"),
    ("Get-ChildItem", "", "", "-Recurse -Force -Filter -Name -File -Directory"),
    ("Remove-Item", "", "", "-Recurse -Force"),
];

const BUILTINS: &str = "alias cat cd clear cls code cp dir echo exit export history ls mkdir mv pwd rm rmdir set source type where which \
    Get-ChildItem Get-Content Get-Location Remove-Item Set-Location";

fn specs(command: &str) -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str, &'static str)> + '_ {
    SPECS.iter().filter(move |s| s.0.split(' ').any(|c| c.eq_ignore_ascii_case(command)))
}

static PATH_COMMANDS: OnceLock<Vec<String>> = OnceLock::new();

pub fn warm() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_ok() {
        std::thread::spawn(|| PATH_COMMANDS.set(scan_path()));
    }
}

fn path_commands() -> &'static [String] {
    warm();
    PATH_COMMANDS.get().map_or(&[], Vec::as_slice)
}

fn scan_path() -> Vec<String> {
    let mut names: Vec<String> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            match name.rsplit_once('.') {
                Some((stem, ext)) if cfg!(windows) => ["exe", "cmd", "bat", "com", "ps1"].contains(&ext.to_lowercase().as_str()).then(|| stem.to_owned()),
                _ if cfg!(windows) => None,
                _ => Some(name),
            }
        })
        .collect();
    names.sort_unstable_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

fn starts(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.is_char_boundary(prefix.len()) && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn quote(text: String, sep: char) -> String {
    match (text.contains(' '), sep) {
        (false, _) => text,
        (true, '/') => text.replace(' ', "\\ "),
        _ => format!("\"{text}\""),
    }
}

fn paths(token: &str, cwd: Option<&Path>, sep: char) -> Vec<Item> {
    let cut = token.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let (dir, prefix) = token.split_at(cut);
    let base: Option<PathBuf> = match dir.strip_prefix('~') {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest.trim_start_matches(['/', '\\']))),
        None if Path::new(dir).is_absolute() || (cfg!(windows) && dir.len() >= 2 && dir.as_bytes()[1] == b':') => Some(PathBuf::from(dir)),
        None => cwd.map(|c| c.join(dir)),
    };
    let Some(Ok(entries)) = base.map(std::fs::read_dir) else { return vec![] };
    let sep = token[..cut].chars().last().unwrap_or(sep);
    let mut items: Vec<Item> = entries
        .flatten()
        .take(5000)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !starts(&name, prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let folder = e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir();
            let label = if folder { format!("{name}{sep}") } else { name };
            let insert = quote(format!("{dir}{label}"), sep);
            Some(Item { label, kind: if folder { Kind::Folder } else { Kind::File }, replace: token.chars().count(), insert })
        })
        .collect();
    items.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase())));
    items
}

pub fn complete(typed: &str, history: &[String], cwd: Option<&Path>, sep: char, forced: bool) -> Vec<Item> {
    let token = typed.rsplit(char::is_whitespace).next().unwrap_or("");
    let words: Vec<&str> = typed[..typed.len() - token.len()].split_whitespace().collect();
    if token.is_empty() && !forced {
        return vec![];
    }
    let replace = token.chars().count();
    let word = |label: &str, kind: Kind| Item { label: label.to_owned(), kind, replace, insert: label.to_owned() };
    let mut items: Vec<Item> = history
        .iter()
        .filter(|line| !typed.trim().is_empty() && starts(line, typed) && line.len() > typed.len())
        .take(3)
        .map(|line| Item { label: line.clone(), kind: Kind::History, replace: typed.chars().count(), insert: line.clone() })
        .collect();
    match words.first() {
        None => {
            let names = SPECS.iter().flat_map(|s| s.0.split(' ')).chain(BUILTINS.split_whitespace()).map(str::to_owned);
            let mut names: Vec<String> = names.chain(path_commands().iter().cloned()).filter(|n| starts(n, token)).collect();
            names.sort_unstable_by_key(|n| n.to_lowercase());
            names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            items.extend(names.iter().map(|n| word(n, Kind::Command)));
            if token.contains(['/', '\\']) || token.starts_with('.') {
                items.extend(paths(token, cwd, sep));
            }
        }
        Some(command) => {
            let sub = words.get(1).copied().unwrap_or("");
            let top: Vec<&str> = specs(command).filter(|s| s.1.is_empty()).flat_map(|s| s.2.split_whitespace()).collect();
            if token.starts_with('-') {
                let mut flags: Vec<&str> = specs(command)
                    .filter(|s| s.1.is_empty() || s.1.split(' ').any(|v| v == sub))
                    .flat_map(|s| s.3.split_whitespace())
                    .filter(|f| starts(f, token))
                    .collect();
                flags.dedup();
                items.extend(flags.iter().map(|f| word(f, Kind::Flag)));
            } else if words.len() == 1 && !top.is_empty() {
                items.extend(top.iter().filter(|s| starts(s, token)).map(|s| word(s, Kind::Sub)));
            } else {
                let nested = specs(command).filter(|s| words.len() == 2 && s.1.split(' ').any(|v| v == sub)).flat_map(|s| s.2.split_whitespace());
                items.extend(nested.filter(|s| starts(s, token)).map(|s| word(s, Kind::Sub)));
                items.extend(paths(token, cwd, sep));
            }
        }
    }
    items.retain(|i| !(i.replace == replace && i.insert == token));
    items.truncate(200);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn completes_commands_subcommands_flags_paths_and_history() {
        let dir = std::env::temp_dir().join(format!("halo-suggest-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src dir")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        let none: [String; 0] = [];
        assert!(complete("git ", &none, None, '/', false).is_empty());
        assert!(labels(&complete("car", &none, None, '/', false)).contains(&"cargo"));
        assert_eq!(labels(&complete("git sta", &none, None, '/', false)), ["stash", "status"]);
        assert_eq!(labels(&complete("git stash p", &none, None, '/', false)), ["pop", "push"]);
        let flags = complete("cargo test --rel", &none, None, '/', false);
        assert_eq!((labels(&flags), flags[0].replace, flags[0].insert.as_str()), (vec!["--release"], 5, "--release"));
        assert!(complete("git commit --amend", &none, None, '/', false).is_empty());
        let files = complete("cat ", &none, Some(&dir), '/', true);
        assert_eq!(labels(&files), ["src dir/", "Cargo.toml"]);
        assert_eq!(files[0].insert, "src\\ dir/");
        let quoted = complete("cd s", &none, Some(&dir), '\\', false);
        assert_eq!((quoted[0].replace, quoted[0].insert.as_str()), (1, "\"src dir\\\""));
        assert_eq!(labels(&complete("cat ./C", &none, Some(&dir), '/', false)), ["Cargo.toml"]);
        let history = ["git status -s".to_owned(), "ls".to_owned()];
        let found = complete("git st", &history, None, '/', false);
        assert_eq!((found[0].kind, found[0].replace, found[0].insert.as_str()), (Kind::History, 6, "git status -s"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
