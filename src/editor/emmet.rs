const TAGS: &str = " a abbr address article aside audio b blockquote body br button canvas caption code col dd div dl dt em fieldset \
    figcaption figure footer form h1 h2 h3 h4 h5 h6 head header hr html i iframe img input label legend li link main meta nav ol optgroup \
    option p pre script section select slot small span strong style sub sup svg table tbody td template textarea tfoot th thead title tr u \
    ul video ";
const VOID: &str = " br col hr img input link meta ";
const PAGE: &str = "<!DOCTYPE html>\n<html lang=\"${1:en}\">\n<head>\n\t<meta charset=\"UTF-8\">\n\t<meta name=\"viewport\" \
    content=\"width=device-width, initial-scale=1.0\">\n\t<title>${2:Document}</title>\n</head>\n<body>\n\t$0\n</body>\n</html>";

#[derive(Default)]
struct Node {
    name: String,
    id: String,
    classes: Vec<String>,
    attrs: Vec<(String, String)>,
    text: String,
    count: usize,
    group: bool,
    children: Vec<Node>,
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        self.at += hit as usize;
        hit
    }

    fn take(&mut self, keep: impl Fn(char) -> bool) -> String {
        let start = self.at;
        while self.peek().is_some_and(&keep) {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect()
    }

    fn until(&mut self, end: char) -> Option<String> {
        self.at += 1;
        let text = self.take(|c| c != end);
        self.eat(end).then_some(text)
    }

    fn repeat(&mut self) -> usize {
        if !self.eat('*') {
            return 1;
        }
        self.take(|c| c.is_ascii_digit()).parse().unwrap_or(1).clamp(1, 200)
    }

    fn element(&mut self) -> Option<Node> {
        if self.eat('(') {
            let children = self.siblings(true)?.0;
            if !self.eat(')') {
                return None;
            }
            return Some(Node { group: true, children, count: self.repeat(), ..Default::default() });
        }
        let word = |c: char| c.is_ascii_alphanumeric() || "-_$@:".contains(c);
        let mut node = Node { name: self.take(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':'), ..Default::default() };
        loop {
            match self.peek() {
                Some('#') => {
                    self.at += 1;
                    node.id = self.take(word);
                }
                Some('.') => {
                    self.at += 1;
                    node.classes.push(self.take(word));
                }
                Some('[') => {
                    for part in self.until(']')?.split_whitespace() {
                        let (key, value) = part.split_once('=').unwrap_or((part, ""));
                        node.attrs.push((key.to_owned(), value.trim_matches(['"', '\'']).to_owned()));
                    }
                }
                Some('{') => node.text = self.until('}')?,
                _ => break,
            }
        }
        let empty = node.name.is_empty() && node.id.is_empty() && node.classes.is_empty() && node.attrs.is_empty() && node.text.is_empty();
        node.count = self.repeat();
        (!empty && node.name.chars().next().is_none_or(|c| c.is_ascii_alphabetic())).then_some(node)
    }

    fn siblings(&mut self, top: bool) -> Option<(Vec<Node>, usize)> {
        let mut nodes = vec![];
        loop {
            let mut node = self.element()?;
            let mut up = 0;
            if self.eat('>') {
                let (children, climb) = self.siblings(false)?;
                up = climb;
                match node.group {
                    true => node.children.last_mut()?.children = children,
                    false => node.children = children,
                }
            }
            nodes.push(node);
            if up > 1 && !top {
                return Some((nodes, up - 1));
            }
            if up > 0 || self.eat('+') {
                continue;
            }
            match self.take(|c| c == '^').len() {
                0 => return Some((nodes, 0)),
                _ if top => {}
                carets => return Some((nodes, carets)),
            }
        }
    }
}

fn numbered(text: &str, n: usize) -> String {
    let (mut out, mut run) = (String::new(), 0);
    for c in text.chars().chain(['\0']) {
        if c == '$' {
            run += 1;
            continue;
        }
        if run > 0 {
            out += &format!("{n:0run$}");
            run = 0;
        }
        if c != '\0' {
            out.push(c);
        }
    }
    out.replace('\\', "\\\\").replace('$', "\\$").replace('}', "\\}")
}

fn implied(parent: &str) -> &'static str {
    match parent {
        "ul" | "ol" => "li",
        "tr" => "td",
        "table" | "tbody" | "thead" | "tfoot" => "tr",
        "select" | "optgroup" => "option",
        "span" | "a" | "em" | "strong" | "b" | "i" | "label" | "small" | "button" => "span",
        _ => "div",
    }
}

#[allow(clippy::too_many_arguments)]
fn render(nodes: &[Node], parent: &str, depth: usize, jsx: bool, outer: usize, stop: &mut usize, out: &mut Vec<String>) {
    for node in nodes {
        for n in 1..=node.count {
            let n = if node.count > 1 { n } else { outer };
            if node.group {
                render(&node.children, parent, depth, jsx, n, stop, out);
                continue;
            }
            let pad = "\t".repeat(depth);
            let text = numbered(&node.text, n);
            if node.name.is_empty() && node.id.is_empty() && node.classes.is_empty() && node.attrs.is_empty() {
                out.push(format!("{pad}{text}"));
                continue;
            }
            let name = if node.name.is_empty() { implied(parent) } else { node.name.as_str() };
            let mut blank = || {
                *stop += 1;
                format!("${{{stop}}}")
            };
            let mut attrs: Vec<(&str, String)> = vec![];
            if !node.id.is_empty() {
                attrs.push(("id", numbered(&node.id, n)));
            }
            if !node.classes.is_empty() {
                attrs.push((if jsx { "className" } else { "class" }, numbered(&node.classes.join(" "), n)));
            }
            for (key, value) in &node.attrs {
                attrs.push((key, if value.is_empty() { blank() } else { numbered(value, n) }));
            }
            let usual: &[&str] = match name {
                "a" | "link" => &["href"],
                "img" => &["src", "alt"],
                "input" => &["type"],
                "form" => &["action"],
                _ => &[],
            };
            for key in usual.iter().filter(|key| !node.attrs.iter().any(|a| a.0 == **key)) {
                attrs.push((key, blank()));
            }
            let attrs: String = attrs.iter().map(|(key, value)| format!(" {key}=\"{value}\"")).collect();
            if VOID.contains(&format!(" {name} ")) {
                out.push(format!("{pad}<{name}{attrs}{}>", if jsx { " /" } else { "" }));
            } else if node.children.is_empty() {
                let inner = if text.is_empty() { blank() } else { text };
                out.push(format!("{pad}<{name}{attrs}>{inner}</{name}>"));
            } else {
                out.push(format!("{pad}<{name}{attrs}>"));
                if !text.is_empty() {
                    out.push(format!("{pad}\t{text}"));
                }
                render(&node.children, name, depth + 1, jsx, n, stop, out);
                out.push(format!("{pad}</{name}>"));
            }
        }
    }
}

fn known(nodes: &[Node]) -> bool {
    nodes.iter().all(|node| (node.name.is_empty() || TAGS.contains(&format!(" {} ", node.name))) && known(&node.children))
}

pub fn abbreviation(before: &str) -> &str {
    let (mut depth, mut start) = (0, before.len());
    for (at, c) in before.char_indices().rev() {
        match c {
            '}' | ']' => depth += 1,
            '{' | '[' => depth -= 1,
            c if c.is_whitespace() && depth <= 0 => break,
            _ => {}
        }
        if depth < 0 {
            break;
        }
        start = at;
    }
    &before[start..]
}

// shortcut: HTML abbreviations only, add the CSS ones (m10, p0) if they are missed
pub fn expand(abbr: &str, jsx: bool, strict: bool) -> Option<String> {
    if abbr == "!" {
        return Some(PAGE.to_owned());
    }
    let mut parser = Parser { chars: abbr.chars().collect(), at: 0 };
    let nodes = parser.siblings(true)?.0;
    if parser.at != parser.chars.len() || (strict && !known(&nodes)) {
        return None;
    }
    let (mut out, mut stop) = (vec![], 0);
    render(&nodes, "", 0, jsx, 1, &mut stop, &mut out);
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_abbreviations() {
        let html = |abbr: &str| expand(abbr, false, true);
        assert_eq!(html("ul#nav>li.item$*2>a{Go $}").unwrap(), "<ul id=\"nav\">\n\t<li class=\"item1\"><a href=\"${1}\">Go 1</a></li>\n\t<li class=\"item2\"><a href=\"${2}\">Go 2</a></li>\n</ul>".replace("<li class=\"item1\"><a", "<li class=\"item1\">\n\t\t<a").replace("</a></li>\n\t<li class=\"item2\"><a", "</a>\n\t</li>\n\t<li class=\"item2\">\n\t\t<a").replace("Go 2</a></li>", "Go 2</a>\n\t</li>"));
        assert_eq!(html("div>p^span+img").unwrap(), "<div>\n\t<p>${1}</p>\n</div>\n<span>${2}</span>\n<img src=\"${3}\" alt=\"${4}\">");
        assert_eq!(html(".a.b+(p>em)*2").unwrap(), "<div class=\"a b\">${1}</div>\n<p>\n\t<em>${2}</em>\n</p>\n<p>\n\t<em>${3}</em>\n</p>");
        assert_eq!(html("input[type=checkbox disabled]").unwrap(), "<input type=\"checkbox\" disabled=\"${1}\">");
        assert_eq!(expand("Card.box>br", true, false).unwrap(), "<Card className=\"box\">\n\t<br />\n</Card>");
        assert_eq!(html("table>.row>.cell").unwrap(), "<table>\n\t<tr class=\"row\">\n\t\t<td class=\"cell\">${1}</td>\n\t</tr>\n</table>");
        assert!(html("!").unwrap().starts_with("<!DOCTYPE html>"));
        for bad in ["console.log", "hello", "div>", "<div>p", "a+", "", "x=1", "(p"] {
            assert_eq!(html(bad), None, "{bad}");
        }
        assert_eq!(abbreviation("  text ul>li{a b}*2"), "ul>li{a b}*2");
        assert_eq!(abbreviation("call(x) "), "");
    }
}
