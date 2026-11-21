use serde_json::Value;

pub fn parse(text: &str) -> Result<Value, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                while let Some(s) = chars.next() {
                    out.push(s);
                    if s == '\\' {
                        out.extend(chars.next());
                    } else if s == '"' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => while chars.next_if(|&n| n != '\n').is_some() {},
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                for n in chars.by_ref() {
                    if previous == '*' && n == '/' {
                        break;
                    }
                    previous = n;
                }
            }
            '}' | ']' => {
                let end = out.trim_end().len();
                if out[..end].ends_with(',') {
                    out.remove(end - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    serde_json::from_str(&out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn strips_comments_and_trailing_commas_outside_strings() {
        assert_eq!(super::parse(r#"{"a": "x\"//y", "b": [1, 2 , ] }"#).unwrap(), json!({"a": "x\"//y", "b": [1, 2]}));
        assert_eq!(super::parse("\u{feff}{ // c\n \"a\": 1, /* b */ \"c\": \"/*\", // d\n }").unwrap(), json!({"a": 1, "c": "/*"}));
        assert!(super::parse("{ not json").is_err());
    }
}
