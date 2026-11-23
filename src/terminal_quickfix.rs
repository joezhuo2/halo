#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    Run(String),
    Open(String),
    FreePort(u16),
}

fn port_in(line: &str) -> Option<u16> {
    let after = |key: &str| line.find(key).map(|i| &line[i + key.len()..]);
    let digits = |text: &str| text.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok().filter(|p| *p > 0);
    after("port ").and_then(digits).or_else(|| line.rmatch_indices(':').find_map(|(i, _)| digits(&line[i + 1..])))
}

pub fn find(line: &str, output: &str) -> Vec<(String, Fix)> {
    let mut fixes = vec![];
    let lines: Vec<&str> = output.lines().map(str::trim).collect();
    if let Some(push) = lines.iter().find(|l| l.starts_with("git push --set-upstream ") || l.starts_with("git push -u ")) {
        fixes.push((format!("Run: {push}"), Fix::Run(push.to_string())));
    }
    if output.contains("is not a git command")
        && let Some(at) = lines.iter().position(|l| l.starts_with("The most similar command"))
    {
        let words: Vec<&str> = line.split_whitespace().collect();
        for similar in lines[at + 1..].iter().take_while(|l| !l.is_empty() && !l.contains(' ')).take(3) {
            let fixed: Vec<&str> = words.iter().enumerate().map(|(i, w)| if i == 1 { *similar } else { *w }).collect();
            let fixed = fixed.join(" ");
            fixes.push((format!("Run: {fixed}"), Fix::Run(fixed)));
        }
    }
    for l in lines.iter().filter(|l| l.starts_with("remote:")) {
        let url = l.split_whitespace().find(|w| w.starts_with("https://") || w.starts_with("http://"));
        if let Some(url) = url.filter(|u| u.contains("/pull/new/") || u.contains("/merge_requests/new") || u.contains("/pull-requests/new")) {
            fixes.push(("Open pull request page".to_owned(), Fix::Open(url.to_owned())));
            break;
        }
    }
    let busy = ["eaddrinuse", "address already in use", "only one usage of each socket address", "port is already allocated"];
    if let Some(port) = lines.iter().filter(|l| busy.iter().any(|b| l.to_lowercase().contains(b))).find_map(|l| port_in(l)) {
        fixes.push((format!("Free port {port}"), Fix::FreePort(port)));
    }
    fixes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_quick_fixes_in_command_output() {
        let upstream = "fatal: The current branch topic has no upstream branch.\nTo push the current branch and set the remote as upstream, use\n\n    git push --set-upstream origin topic\n";
        assert_eq!(find("git push", upstream), vec![("Run: git push --set-upstream origin topic".into(), Fix::Run("git push --set-upstream origin topic".into()))]);
        let typo = "git: 'stauts' is not a git command. See 'git --help'.\n\nThe most similar command is\n\tstatus\n";
        assert_eq!(find("git stauts -s", typo), vec![("Run: git status -s".into(), Fix::Run("git status -s".into()))]);
        let pr = "remote: Create a pull request for 'topic' on GitHub by visiting:\nremote:      https://github.com/o/r/pull/new/topic\nremote:\n";
        assert_eq!(find("git push -u origin topic", pr), vec![("Open pull request page".into(), Fix::Open("https://github.com/o/r/pull/new/topic".into()))]);
        let busy = "Error: listen EADDRINUSE: address already in use :::3000\n    at Server.setupListenHandle";
        assert_eq!(find("npm start", busy), vec![("Free port 3000".into(), Fix::FreePort(3000))]);
        assert_eq!(port_in("OSError: [Errno 98] Address already in use on port 8000"), Some(8000));
        assert!(find("ls", "a\nb\n").is_empty());
    }
}
