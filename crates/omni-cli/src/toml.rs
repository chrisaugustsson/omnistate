//! The small part of TOML that the CLI's two files use: `key = "value"` lines,
//! `[table]` headers, comments and blank lines. Values are strings (basic or
//! literal) or a bare word such as a number or `true`; arrays and nested or
//! dotted tables are not read.

/// The `key = value` pairs of `text`, in order. A key under a `[table]` header
/// is returned as `table.key`; keys before any header stay as they are. The
/// error says what is wrong and on which line.
pub fn parse(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut table = String::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        if let Some(header) = line.strip_prefix('[') {
            let name = header
                .split_once(']')
                .filter(|(_, after)| tail(after, String::new()).is_ok())
                .map(|(name, _)| name.trim())
                .filter(|name| plain(name))
                .ok_or_else(|| format!("line {number}: {line:?} is not a plain table header"))?;
            if pairs
                .iter()
                .any(|(k, _)| k.starts_with(&format!("{name}.")))
            {
                return Err(format!("line {number}: table {name} is set twice"));
            }
            table = name.to_string();
            continue;
        }
        let (key, rest) = if let Some(quoted) = line.strip_prefix('"') {
            let (key, after) = basic_string(quoted).map_err(|e| format!("line {number}: {e}"))?;
            let rest = after
                .trim_start()
                .strip_prefix('=')
                .ok_or_else(|| format!("line {number}: expected key = \"value\""))?;
            (key, rest)
        } else {
            let (key, rest) = line
                .split_once('=')
                .ok_or_else(|| format!("line {number}: expected key = \"value\""))?;
            let key = key.trim();
            if !plain(key) {
                return Err(format!("line {number}: {key:?} is not a plain key"));
            }
            (key.to_string(), rest)
        };
        let key = &if table.is_empty() {
            key
        } else {
            format!("{table}.{key}")
        };
        if pairs.iter().any(|(k, _)| k == key) {
            return Err(format!("line {number}: {key} is set twice"));
        }
        let value = value(rest.trim()).map_err(|e| format!("line {number}: {e}"))?;
        pairs.push((key.to_string(), value));
    }
    Ok(pairs)
}

fn plain(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn value(text: &str) -> Result<String, String> {
    let mut chars = text.chars();
    match chars.next() {
        None => Err("the value is missing".to_string()),
        Some('"') => {
            let (out, rest) = basic_string(chars.as_str())?;
            tail(rest, out)
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

/// The basic string that starts after an opening quote, and the text after its
/// closing quote.
fn basic_string(text: &str) -> Result<(String, &str), String> {
    let mut chars = text.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Ok((out, chars.as_str())),
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
            "a = \"x\" y\n",
            "a = \"open\n",
            "a = 1\na = 2\n",
            "= 1\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_table_header_names_the_keys_under_it_and_leaves_the_keys_before_it_flat() {
        let text = "project = \"p\"\nfeature = \"f\"\n[checks]\nfast = \"a\"\nstack = \"c\"\n";
        let pairs = parse(text).unwrap();
        let expected = [
            ("project", "p"),
            ("feature", "f"),
            ("checks.fast", "a"),
            ("checks.stack", "c"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(pairs, expected);
    }

    #[test]
    fn quoted_strings_read_back() {
        let text = format!("k = {}\n", quote("a \"b\" \\ c\nd"));
        assert_eq!(
            get(&parse(&text).unwrap(), "k").as_deref(),
            Some("a \"b\" \\ c\nd")
        );
    }

    #[test]
    fn a_quoted_key_is_read_unescaped_and_prefixed_by_its_table() {
        let pairs = parse("\"/Users/me/app\" = \"01a1\"").unwrap();
        assert_eq!(pairs, [("/Users/me/app".to_string(), "01a1".to_string())]);
        let pairs = parse("[ws]\n\"/a b\" = \"x\"").unwrap();
        assert_eq!(pairs, [("ws./a b".to_string(), "x".to_string())]);
        let pairs = parse("\"a\\\"b\" = \"x\"").unwrap();
        assert_eq!(pairs, [("a\"b".to_string(), "x".to_string())]);
    }

    #[test]
    fn a_key_written_by_quote_reads_back_even_with_an_equals_sign_and_a_hash() {
        let text = format!("{} = \"x\"\n", quote("/Users/me/a=b #c"));
        assert_eq!(
            parse(&text).unwrap(),
            [("/Users/me/a=b #c".to_string(), "x".to_string())]
        );
    }

    #[test]
    fn a_bare_key_with_a_slash_and_a_quoted_table_header_stay_refused() {
        let err = parse("/a = \"x\"").unwrap_err();
        assert!(
            err.contains("line 1") && err.contains("is not a plain key"),
            "{err}"
        );
        assert!(parse("[\"a\"]\n").is_err());
    }
}
