//! The small part of TOML that the CLI's two files use: `key = "value"` lines,
//! comments and blank lines. Values are strings (basic or literal) or a bare
//! word such as a number or `true`; tables and arrays are not read.

/// The `key = value` pairs of `text`, in order. The error says what is wrong
/// and on which line.
pub fn parse(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let (key, rest) = line
            .split_once('=')
            .ok_or_else(|| format!("line {number}: expected key = \"value\""))?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("line {number}: {key:?} is not a plain key"));
        }
        if pairs.iter().any(|(k, _)| k == key) {
            return Err(format!("line {number}: {key} is set twice"));
        }
        let value = value(rest.trim()).map_err(|e| format!("line {number}: {e}"))?;
        pairs.push((key.to_string(), value));
    }
    Ok(pairs)
}

fn value(text: &str) -> Result<String, String> {
    let mut chars = text.chars();
    match chars.next() {
        None => Err("the value is missing".to_string()),
        Some('"') => {
            let mut out = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return tail(chars.as_str(), out),
                    '\\' => out.push(match chars.next() {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('"') => '"',
                        Some('\\') => '\\',
                        _ => return Err("unknown escape in the string".to_string()),
                    }),
                    c => out.push(c),
                }
            }
            Err("the string is not closed".to_string())
        }
        Some('\'') => {
            let rest = chars.as_str();
            let end = rest
                .find('\'')
                .ok_or_else(|| "the string is not closed".to_string())?;
            tail(&rest[end + 1..], rest[..end].to_string())
        }
        Some(_) => {
            let word = text.split('#').next().unwrap_or("").trim();
            if word.is_empty() || word.contains(char::is_whitespace) {
                return Err("expected a string in quotes".to_string());
            }
            Ok(word.to_string())
        }
    }
}

/// Nothing but a comment may follow a value.
fn tail(rest: &str, value: String) -> Result<String, String> {
    let rest = rest.trim();
    if rest.is_empty() || rest.starts_with('#') {
        Ok(value)
    } else {
        Err("unexpected text after the value".to_string())
    }
}

/// `text` as a TOML basic string.
pub fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(pairs: &[(String, String)], key: &str) -> Option<String> {
        pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn reads_strings_words_and_comments() {
        let text = "# c\nproject = \"brand-site\" # note\nname = 'lit'\nn = 3\n\nq = \"a\\\"b\"\n";
        let pairs = parse(text).unwrap();
        assert_eq!(get(&pairs, "project").as_deref(), Some("brand-site"));
        assert_eq!(get(&pairs, "name").as_deref(), Some("lit"));
        assert_eq!(get(&pairs, "n").as_deref(), Some("3"));
        assert_eq!(get(&pairs, "q").as_deref(), Some("a\"b"));
    }

    #[test]
    fn rejects_what_it_cannot_read() {
        for bad in [
            "project = \n",
            "project",
            "[table]\n",
            "a = \"x\" y\n",
            "a = \"open\n",
            "a = 1\na = 2\n",
            "= 1\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn quoted_strings_read_back() {
        let text = format!("k = {}\n", quote("a \"b\" \\ c\nd"));
        assert_eq!(
            get(&parse(&text).unwrap(), "k").as_deref(),
            Some("a \"b\" \\ c\nd")
        );
    }
}
