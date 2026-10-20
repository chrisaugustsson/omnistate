//! A small JSON reader and writer. The CLI reads a few fields out of API
//! responses and builds a few request bodies, and nothing else, so this is all
//! it needs and the binary stays free of dependencies and starts instantly.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as written, so nothing is lost by reading it.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// How deep arrays and objects may nest before the text is refused.
const MAX_DEPTH: usize = 64;

impl Value {
    pub fn parse(text: &str) -> Result<Value, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            at: 0,
        };
        let value = parser.value(0)?;
        parser.skip_space();
        if parser.at != parser.bytes.len() {
            return Err(parser.error("unexpected text after the value"));
        }
        Ok(value)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The string at `key`, if this is an object with a string there.
    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// The value written over several lines, indented by two spaces, for a
    /// file that a person or an agent reads.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }

    fn write_pretty(&self, out: &mut String, depth: usize) {
        let indent = |out: &mut String, depth: usize| out.push_str(&"  ".repeat(depth));
        match self {
            Value::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    indent(out, depth + 1);
                    item.write_pretty(out, depth + 1);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                indent(out, depth);
                out.push(']');
            }
            Value::Object(fields) if !fields.is_empty() => {
                out.push_str("{\n");
                for (i, (key, value)) in fields.iter().enumerate() {
                    indent(out, depth + 1);
                    out.push_str(&quote(key));
                    out.push_str(": ");
                    value.write_pretty(out, depth + 1);
                    out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
                }
                indent(out, depth);
                out.push('}');
            }
            other => out.push_str(&other.to_string()),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(n) => f.write_str(n),
            Value::String(s) => f.write_str(&quote(s)),
            Value::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Value::Object(fields) => {
                f.write_str("{")?;
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{}:{value}", quote(key))?;
                }
                f.write_str("}")
            }
        }
    }
}

/// `text` as a JSON string literal.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.at)
    }

    fn skip_space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid value"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        self.skip_space();
        match self.bytes.get(self.at) {
            None => Err(self.error("unexpected end")),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.skip_space();
                if self.eat(b']') {
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.skip_space();
                    if self.eat(b']') {
                        return Ok(Value::Array(items));
                    }
                    if !self.eat(b',') {
                        return Err(self.error("expected , or ]"));
                    }
                }
            }
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                self.skip_space();
                if self.eat(b'}') {
                    return Ok(Value::Object(fields));
                }
                loop {
                    self.skip_space();
                    if self.bytes.get(self.at) != Some(&b'"') {
                        return Err(self.error("expected a key"));
                    }
                    let key = self.string()?;
                    self.skip_space();
                    if !self.eat(b':') {
                        return Err(self.error("expected :"));
                    }
                    fields.push((key, self.value(depth + 1)?));
                    self.skip_space();
                    if self.eat(b'}') {
                        return Ok(Value::Object(fields));
                    }
                    if !self.eat(b',') {
                        return Err(self.error("expected , or }"));
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while matches!(
                    self.bytes.get(self.at),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.at += 1;
                }
                // The slice is ASCII, so it is valid UTF-8.
                let number = String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned();
                Ok(Value::Number(number))
            }
            Some(_) => Err(self.error("invalid value")),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u32::from_str_radix(d, 16).ok())
            .ok_or_else(|| self.error("invalid \\u escape"))?;
        self.at += 4;
        Ok(digits)
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1; // the opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(self.error("unterminated string"));
            };
            self.at += 1;
            match byte {
                b'"' => {
                    return String::from_utf8(out).map_err(|_| self.error("invalid UTF-8"));
                }
                b'\\' => {
                    let Some(&escape) = self.bytes.get(self.at) else {
                        return Err(self.error("unterminated string"));
                    };
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first) {
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return Err(self.error("unpaired surrogate"));
                                }
                                let second = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return Err(self.error("unpaired surrogate"));
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else {
                                first
                            };
                            char::from_u32(code).ok_or_else(|| self.error("invalid \\u escape"))?
                        }
                        _ => return Err(self.error("invalid escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(byte),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_values_and_escapes() {
        let text = r#" {"a": [1, 2.5e3, true, null], "s": "x\"\u00e9\ud83d\ude00\n", "o": {}} "#;
        let value = Value::parse(text).unwrap();
        assert_eq!(value.get("s").and_then(Value::as_str), Some("x\"é😀\n"));
        assert_eq!(value.get("a").and_then(Value::as_array).unwrap().len(), 4);
        assert_eq!(value.get("o"), Some(&Value::Object(Vec::new())));
        assert_eq!(value.get("missing"), None);
    }

    #[test]
    fn rejects_malformed_text() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "\"abc",
            "1 2",
            "nul",
            "\"\\ud800\"",
        ] {
            assert!(Value::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Value::parse(&"[".repeat(200)).is_err());
    }

    #[test]
    fn pretty_writing_reads_back_as_the_same_value() {
        let text = r#"{"k":["a\n\"b\"",1,false,null,{"x":"é","e":{},"l":[]}]}"#;
        let value = Value::parse(text).unwrap();
        let pretty = value.pretty();
        assert!(pretty.lines().count() > 5, "{pretty}");
        assert_eq!(Value::parse(&pretty).unwrap(), value);
        assert_eq!(Value::Object(Vec::new()).pretty(), "{}");
    }

    #[test]
    fn writes_what_it_reads() {
        let text = r#"{"k":["a\n\"b\"",1,false,null,{"x":"é"}]}"#;
        let value = Value::parse(text).unwrap();
        assert_eq!(value.to_string(), text);
        assert_eq!(quote("a\u{1}b"), "\"a\\u0001b\"");
    }
}
