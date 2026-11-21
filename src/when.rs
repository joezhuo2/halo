use fancy_regex::Regex;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub enum Expr {
    Const(bool),
    Key(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Eq(String, String, bool),
    Matches(String, Regex),
    In(String, String, bool),
}

#[derive(Debug, PartialEq, Clone)]
enum Token {
    Not,
    And,
    Or,
    Open,
    Close,
    Eq,
    Ne,
    Match,
    Regex(String, String),
    Word(String),
    Quoted(String),
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut i) = (vec![], 0);
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let two = |a, b| c == a && next == Some(b);
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let (token, len) = if two('&', '&') {
            (Token::And, 2)
        } else if two('|', '|') {
            (Token::Or, 2)
        } else if two('=', '=') {
            (Token::Eq, 2 + (chars.get(i + 2) == Some(&'=')) as usize)
        } else if two('!', '=') {
            (Token::Ne, 2 + (chars.get(i + 2) == Some(&'=')) as usize)
        } else if two('=', '~') {
            (Token::Match, 2)
        } else if c == '!' {
            (Token::Not, 1)
        } else if c == '(' {
            (Token::Open, 1)
        } else if c == ')' {
            (Token::Close, 1)
        } else if c == '/' && out.last() == Some(&Token::Match) {
            let mut j = i + 1;
            let mut pattern = String::new();
            while j < chars.len() && chars[j] != '/' {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    if chars[j + 1] != '/' {
                        pattern.push('\\');
                    }
                    j += 1;
                }
                pattern.push(chars[j]);
                j += 1;
            }
            if j >= chars.len() {
                return Err(format!("the regular expression at column {} is not closed", i + 1));
            }
            let flags: String = chars[j + 1..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
            (Token::Regex(pattern, flags.clone()), j + 1 - i + flags.len())
        } else if c == '\'' || c == '"' {
            let end = chars[i + 1..].iter().position(|&q| q == c).ok_or_else(|| format!("the quote at column {} is not closed", i + 1))?;
            (Token::Quoted(chars[i + 1..i + 1 + end].iter().collect()), end + 2)
        } else {
            let len = chars[i..].iter().take_while(|c| !c.is_whitespace() && !"!&|()=".contains(**c)).count();
            if len == 0 {
                return Err(format!("unexpected '{c}' at column {}", i + 1));
            }
            (Token::Word(chars[i..i + len].iter().collect()), len)
        };
        out.push(token);
        i += len;
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        self.at += 1;
        self.tokens.get(self.at - 1).cloned()
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.peek() == Some(&Token::Or) {
            self.at += 1;
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Token::And) {
            self.at += 1;
            left = Expr::And(Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Not) => Ok(Expr::Not(Box::new(self.unary()?))),
            Some(Token::Open) => {
                let inner = self.or()?;
                match self.next() {
                    Some(Token::Close) => Ok(inner),
                    _ => Err("a '(' is not closed".into()),
                }
            }
            Some(Token::Word(word)) if word == "true" || word == "false" => Ok(Expr::Const(word == "true")),
            Some(Token::Word(key)) => self.compare(key),
            Some(other) => Err(format!("unexpected {other:?}")),
            None => Err("the expression ends too early".into()),
        }
    }

    fn value(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token::Word(word) | Token::Quoted(word)) => Ok(word),
            _ => Err("a value is missing after the operator".into()),
        }
    }

    fn compare(&mut self, key: String) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Token::Eq | Token::Ne) => {
                let equal = self.next() == Some(Token::Eq);
                Ok(Expr::Eq(key, self.value()?, equal))
            }
            Some(Token::Match) => {
                self.at += 1;
                let Some(Token::Regex(pattern, flags)) = self.next() else { return Err("=~ needs a /regular expression/".into()) };
                let inline: String = flags.chars().filter(|f| "imsx".contains(*f)).collect();
                let source = if inline.is_empty() { pattern } else { format!("(?{inline}){pattern}") };
                Regex::new(&source).map(|regex| Expr::Matches(key, regex)).map_err(|e| format!("bad regular expression: {e}"))
            }
            Some(Token::Word(word)) if word == "in" => {
                self.at += 1;
                Ok(Expr::In(key, self.value()?, true))
            }
            Some(Token::Word(word)) if word == "not" && self.tokens.get(self.at + 1) == Some(&Token::Word("in".into())) => {
                self.at += 2;
                Ok(Expr::In(key, self.value()?, false))
            }
            _ => Ok(Expr::Key(key)),
        }
    }
}

pub fn parse(text: &str) -> Result<Expr, String> {
    let mut parser = Parser { tokens: tokens(text)?, at: 0 };
    let expr = parser.or()?;
    match parser.peek() {
        None => Ok(expr),
        Some(token) => Err(format!("unexpected {token:?} after the expression")),
    }
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

impl Expr {
    pub fn eval(&self, keys: &Map<String, Value>) -> bool {
        match self {
            Expr::Const(b) => *b,
            Expr::Key(key) => truthy(keys.get(key)),
            Expr::Not(inner) => !inner.eval(keys),
            Expr::And(a, b) => a.eval(keys) && b.eval(keys),
            Expr::Or(a, b) => a.eval(keys) || b.eval(keys),
            Expr::Eq(key, value, equal) => (text(keys.get(key)) == *value) == *equal,
            Expr::Matches(key, regex) => regex.is_match(&text(keys.get(key))).unwrap_or(false),
            Expr::In(key, list, inside) => {
                let item = text(keys.get(key));
                let found = match keys.get(list) {
                    Some(Value::Array(items)) => items.iter().any(|v| text(Some(v)) == item),
                    Some(Value::Object(map)) => map.contains_key(&item),
                    Some(Value::String(s)) => s.split(',').any(|part| part.trim() == item),
                    _ => false,
                };
                found == *inside
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_and_evaluates_every_operator() {
        let keys = json!({
            "editorTextFocus": true, "terminalFocus": false, "editorLangId": "rust", "resourceFilename": "Cargo.toml",
            "debugState": "stopped", "count": 0, "known": ["a.rs", "Cargo.toml"], "map": {"rust": 1}, "empty": "",
        });
        let keys = keys.as_object().unwrap();
        let eval = |text: &str| parse(text).unwrap_or_else(|e| panic!("{text}: {e}")).eval(keys);
        assert!(eval("editorTextFocus"));
        assert!(!eval("terminalFocus") && !eval("missing") && !eval("count") && !eval("empty"));
        assert!(eval("!terminalFocus") && eval("!!editorTextFocus"));
        assert!(eval("editorTextFocus && !terminalFocus") && !eval("editorTextFocus && terminalFocus"));
        assert!(eval("terminalFocus || editorTextFocus") && !eval("terminalFocus || missing"));
        assert!(eval("editorLangId == rust") && eval("editorLangId == 'rust'") && eval("editorLangId === \"rust\""));
        assert!(eval("editorLangId != python") && !eval("editorLangId != rust"));
        assert!(eval("debugState == stopped && editorLangId=='rust'"));
        assert!(eval("resourceFilename =~ /^cargo\\.toml$/i") && !eval("resourceFilename =~ /^cargo/"));
        assert!(eval("resourceFilename =~ /\\.toml$/") && eval("resourceFilename =~ /o\\/?m/"));
        assert!(eval("resourceFilename in known") && !eval("editorLangId in known") && eval("editorLangId in map"));
        assert!(eval("editorLangId not in known") && !eval("resourceFilename not in known"));
        assert!(eval("terminalFocus && editorTextFocus || editorLangId == rust"));
        assert!(!eval("terminalFocus && (editorTextFocus || editorLangId == rust)"));
        assert!(eval("!(terminalFocus || missing) && true") && !eval("false"));
        for bad in ["", "a &&", "(a", "a == ", "a =~ rust", "a =~ /x", "a )", "a == 'x", "a || && b", "a =~ /(/"] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }
}
