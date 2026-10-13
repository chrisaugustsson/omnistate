//! SVG path data.

use super::json::parse_str;
use super::value::InvalidValue;
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;

/// SVG path data, such as `M0 0 L24 24`, kept exactly as written. Vectors hold
/// path data only, never SVG markup.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct PathData(String);

impl PathData {
    /// The path data as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PathData {
    type Err = InvalidValue;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        check(path)
            .map(|()| PathData(path.to_string()))
            .map_err(|problem| InvalidValue(format!("invalid SVG path data: {problem}")))
    }
}

impl<'de> Deserialize<'de> for PathData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        parse_str(deserializer, "SVG path data such as `M0 0 L24 24`")
    }
}

/// An argument of a path command.
#[derive(Clone, Copy)]
enum Argument {
    Number,
    /// An arc flag: `0` or `1`, which needs no separator before what follows.
    Flag,
}

use Argument::{Flag, Number};

/// The arguments of one use of `command`, or `None` if it is not a command.
fn arguments(command: u8) -> Option<&'static [Argument]> {
    match command.to_ascii_lowercase() {
        b'm' | b'l' | b't' => Some(&[Number, Number]),
        b'h' | b'v' => Some(&[Number]),
        b'c' => Some(&[Number, Number, Number, Number, Number, Number]),
        b's' | b'q' => Some(&[Number, Number, Number, Number]),
        b'a' => Some(&[Number, Number, Number, Flag, Flag, Number, Number]),
        b'z' => Some(&[]),
        _ => None,
    }
}

/// Checks `path` against the SVG path data grammar: a moveto first, then
/// commands, each followed by one or more groups of its arguments.
fn check(path: &str) -> Result<(), String> {
    let mut scanner = Scanner { path, at: 0 };
    scanner.skip_spaces();
    if scanner.at_end() {
        return Err("the path is empty".to_string());
    }
    let mut first = true;
    while !scanner.at_end() {
        let command = scanner.command(first)?;
        first = false;
        if !command.is_empty() {
            scanner.argument_groups(command)?;
        }
        scanner.skip_spaces();
    }
    Ok(())
}

struct Scanner<'a> {
    path: &'a str,
    at: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.path.as_bytes().get(self.at).copied()
    }

    fn at_end(&self) -> bool {
        self.at == self.path.len()
    }

    fn error<T>(&self, expected: &str) -> Result<T, String> {
        // Scanning stops at the first byte of a character it does not know,
        // so `at` is always on a character boundary.
        let position = self.path[..self.at].chars().count() + 1;
        Err(format!("expected {expected} at character {position}"))
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')) {
            self.at += 1;
        }
    }

    /// Skips what separates two arguments: spaces, at most one comma, and
    /// spaces. Returns whether there was a comma.
    fn skip_separator(&mut self) -> bool {
        self.skip_spaces();
        let comma = self.peek() == Some(b',');
        if comma {
            self.at += 1;
            self.skip_spaces();
        }
        comma
    }

    fn command(&mut self, first: bool) -> Result<&'static [Argument], String> {
        let letter = self.peek().unwrap_or_default();
        match arguments(letter) {
            Some(_) if first && !matches!(letter, b'M' | b'm') => {
                self.error("a moveto (`M` or `m`) to start the path")
            }
            Some(command) => {
                self.at += 1;
                Ok(command)
            }
            None => self.error("a path command"),
        }
    }

    /// Reads one or more groups of a command's arguments, as in `L10 10 20 20`.
    fn argument_groups(&mut self, command: &[Argument]) -> Result<(), String> {
        loop {
            for (index, argument) in command.iter().enumerate() {
                if index == 0 {
                    self.skip_spaces();
                } else {
                    self.skip_separator();
                }
                match argument {
                    Number => self.number()?,
                    Flag => self.flag()?,
                }
            }
            let comma = self.skip_separator();
            if !matches!(self.peek(), Some(b'0'..=b'9' | b'+' | b'-' | b'.')) {
                return if comma {
                    self.error("a number after `,`")
                } else {
                    Ok(())
                };
            }
        }
    }

    /// Reads a number: an optional sign, digits with an optional fraction or a
    /// fraction alone, and an optional exponent.
    fn number(&mut self) -> Result<(), String> {
        let start = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let mut digits = self.digits();
        if self.peek() == Some(b'.') {
            self.at += 1;
            digits += self.digits();
        }
        if digits == 0 {
            self.at = start;
            return self.error("a number");
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return self.error("the digits of an exponent");
            }
        }
        Ok(())
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn flag(&mut self) -> Result<(), String> {
        match self.peek() {
            Some(b'0' | b'1') => {
                self.at += 1;
                Ok(())
            }
            _ => self.error("an arc flag (`0` or `1`)"),
        }
    }
}
