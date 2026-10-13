//! Where a line of text may break: the soft wrap opportunities that Chrome
//! finds in text with CSS `white-space: pre-wrap`, so the renderer breaks
//! lines where a browser breaks them.

use unicode_linebreak::BreakClass::{
    Alphabetic as AL, Ambiguous as AI, CarriageReturn as CR, ComplexContext as SA,
    Contingent as CB, HebrewLetter as HL, LineFeed as LF, Mandatory as BK, NextLine as NL,
    NonBreakingGlue as GL, Space as SP, Surrogate as SG, Unknown as XX, ZeroWidthSpace as ZW,
};
use unicode_linebreak::{break_property, linebreaks};

/// The offsets, in bytes, at which a line of `text` may break before the
/// character there, in increasing order. A line break (`\n`) always ends its
/// line, so the offsets just after line breaks are left out.
///
/// The rules are Chrome's, which follow Unicode's line breaking algorithm
/// (UAX #14) only next to characters beyond ASCII:
///
/// - A line never breaks before a space or a tab, and may break after a run
///   of them, which then stays at the end of the line.
/// - Between two other ASCII characters, a line breaks only after a hyphen or
///   a question mark, and before an opening bracket that follows closing
///   punctuation or most other symbols. Between a hyphen and a digit it
///   breaks only when a letter or a digit comes before the hyphen, so that a
///   minus sign stays with its number. Paths, URLs and abbreviations such as
///   `input/output` and `e.g.` stay whole.
/// - Next to any other character, except a no-break space, a line breaks
///   where UAX #14 allows it: before and after an em dash, after an ellipsis
///   or an en dash, and between ideographs, for example. As in Unicode 15.1
///   and later, a hyphen or an en dash that starts a word stays with it.
pub(super) fn soft_breaks(text: &str) -> Vec<usize> {
    let unicode_breaks: Vec<usize> = linebreaks(text).map(|(offset, _)| offset).collect();
    let mut breaks = Vec::new();
    let mut chars = text.char_indices();
    let Some((_, mut last)) = chars.next() else {
        return breaks;
    };
    let mut before_last = None;
    for (offset, next) in chars {
        let unicode_allows = unicode_breaks.binary_search(&offset).is_ok();
        if last != '\n' && breaks_between(before_last, last, next, unicode_allows) {
            breaks.push(offset);
        }
        before_last = Some(last);
        last = next;
    }
    breaks
}

/// Whether a line may break between `last` and `next`, where `before_last`
/// precedes `last` and `unicode_allows` says whether UAX #14 allows the break.
fn breaks_between(before_last: Option<char>, last: char, next: char, unicode_allows: bool) -> bool {
    if is_space(next) {
        false
    } else if is_space(last) {
        true
    } else if last == '-' && next.is_ascii_digit() {
        before_last.is_some_and(|before| before.is_ascii_alphanumeric())
    } else if last.is_ascii_graphic() && next.is_ascii_graphic() {
        ascii_breaks_between(last, next)
    } else {
        unicode_allows
            && (beyond_ascii(last) || beyond_ascii(next))
            && !starts_word_with_hyphen(before_last, last, next)
    }
}

/// Whether `last` is a hyphen or a dash at the start of a word, before the
/// letter `next`. UAX #14 has kept them together since Unicode 15.1 (rule
/// LB20a), and Chrome does; the UAX #14 of `unicode_linebreak` predates the
/// rule. A word starts at the start of the text, or after a line break, a
/// space or a character that never breaks.
fn starts_word_with_hyphen(before_last: Option<char>, last: char, next: char) -> bool {
    // The hyphens of the Unicode 16 line breaking classes HY and HH.
    const HYPHENS: &str =
        "-\u{58a}\u{5be}\u{1400}\u{2010}\u{2012}\u{2013}\u{2e17}\u{2e40}\u{2e5d}\u{10ead}";
    let starts_word = before_last.is_none_or(|before| {
        is_space(before)
            || matches!(
                break_property(before.into()),
                BK | CR | LF | NL | SP | ZW | CB | GL
            )
    });
    let letter = matches!(break_property(next.into()), AL | HL | AI | SA | SG | XX);
    HYPHENS.contains(last) && starts_word && letter
}

/// Whether a line may break between the ASCII characters `last` and `next`,
/// neither a space: Blink's table for ASCII text, which breaks after a hyphen
/// or a question mark, and before an opening bracket after closing
/// punctuation and most symbols.
fn ascii_breaks_between(last: char, next: char) -> bool {
    match last {
        // Digits after a hyphen are left to `breaks_between`.
        '-' => !matches!(
            next,
            '!' | '$' | ')' | ',' | '.' | '/' | '0'..='9' | ':' | ';' | '?' | ']' | '}'
        ),
        '?' => !matches!(
            next,
            '!' | '"' | '\'' | ')' | ',' | '.' | '/' | ':' | ';' | '?' | ']' | '}'
        ),
        '!' | '"' | '#' | '%' | '&' | ')' | '*' | '+' | ',' | '.' | ':' | ';' | '=' | '>'
        | '\\' | ']' | '|' | '}' | '~' => matches!(next, '(' | '<' | '[' | '{'),
        _ => false,
    }
}

/// Spaces, tabs and line breaks, which a line never breaks before.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}

/// A character whose breaks UAX #14 decides: one beyond ASCII, except a
/// no-break space, which never breaks next to ASCII.
fn beyond_ascii(c: char) -> bool {
    c > '\u{7f}' && c != '\u{a0}'
}
