//! Plain-text scanning shared by `whitespace` mode and HTML text runs.

use std::ops::Range;

use super::{Builder, Edges};

/// Byte order mark. Skipped at the start of a document; it is not text.
const BOM: &str = "\u{feff}";

/// Offset of the first byte to scan. A document may open with a byte order
/// mark. A fragment cut from inside a document never does, so there the same
/// character is ordinary text.
pub(super) fn start_offset(source: &str, builder: &Builder) -> usize {
    if builder.edges == Edges::Document && source.starts_with(BOM) {
        BOM.len()
    } else {
        0
    }
}

/// Scan a whole plain-text input.
pub(super) fn scan(source: &str, builder: &mut Builder) {
    let start = start_offset(source, builder);
    scan_range(source, start..source.len(), builder, |_, _, _| None);
}

/// True when `line` opens with a list marker followed by a space: a bullet
/// (`-`, `*`, `+`, `•`), a number (`1.`, `12)`) or a parenthesised label
/// (`(a)`, `(iv)`).
fn starts_list_item(line: &str) -> bool {
    let after_marker = if let Some(rest) = line.strip_prefix(['-', '*', '+', '\u{2022}']) {
        rest
    } else if let Some(rest) = line.strip_prefix('(') {
        let label = rest.bytes().take_while(u8::is_ascii_alphanumeric).count();
        match rest[label..].strip_prefix(')') {
            Some(rest) if (1..=4).contains(&label) => rest,
            _ => return false,
        }
    } else {
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        match line[digits..].strip_prefix(['.', ')']) {
            Some(rest) if (1..=3).contains(&digits) => rest,
            _ => return false,
        }
    };
    after_marker.starts_with([' ', '\t'])
}

/// Scan `source[range]` as text: whitespace runs become separators, the rest
/// is copied verbatim.
///
/// `special` is asked at every non-whitespace character whether something
/// other than literal text starts there. It returns the decoded replacement
/// and the offset just past it. HTML mode uses it for character references.
pub(super) fn scan_range(
    source: &str,
    range: Range<usize>,
    builder: &mut Builder,
    special: impl Fn(&str, usize, usize) -> Option<(String, usize)>,
) {
    let limit = range.end;
    let mut pos = range.start;
    let mut run_start = pos;
    while pos < limit {
        let ch = match source[pos..limit].chars().next() {
            Some(ch) => ch,
            None => break,
        };
        if ch.is_whitespace() {
            builder.verbatim(source, run_start..pos);
            let ws_start = pos;
            let mut breaks = 0u32;
            while pos < limit {
                let Some(ws) = source[pos..limit].chars().next() else {
                    break;
                };
                if !ws.is_whitespace() {
                    break;
                }
                let next = pos + ws.len_utf8();
                match ws {
                    '\n' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' => breaks += 1,
                    '\r' if !source[next..limit].starts_with('\n') => breaks += 1,
                    '\u{2029}' => breaks += 2,
                    _ => {}
                }
                pos = next;
            }
            // A blank line starts a new block. So does a line that opens with
            // a list marker: each item of a list is its own block.
            let block = breaks >= 2 || (breaks == 1 && starts_list_item(&source[pos..limit]));
            builder.separator(ws_start..pos, block);
            run_start = pos;
            continue;
        }
        if let Some((decoded, end)) = special(source, pos, limit) {
            builder.verbatim(source, run_start..pos);
            if decoded.chars().all(char::is_whitespace) {
                builder.separator(pos..end, false);
            } else {
                builder.mapped(&decoded, pos..end);
            }
            pos = end;
            run_start = pos;
            continue;
        }
        pos += ch.len_utf8();
    }
    builder.verbatim(source, run_start..pos);
}
