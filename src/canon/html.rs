//! HTML and XHTML text extraction that keeps byte positions.
//!
//! This is a scanner, not a tree builder. It finds markup boundaries the way
//! the WHATWG tokenizer does (quoted attribute values, comments, raw-text
//! elements) and hands everything else to the text scanner. A DOM parser
//! would be more forgiving of broken nesting, but it would also discard the
//! byte offsets this tool exists to preserve.

use super::Builder;
use super::entities;
use super::text;

/// Elements whose start or end marks a boundary between blocks of text.
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "caption",
    "center",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "option",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
];

/// Elements whose content is dropped whole, up to the matching end tag.
/// `script` and `style` hold code. `ix:header` is the hidden Inline XBRL
/// block (contexts, units, hidden facts) that a reader never sees.
const DROPPED: &[&str] = &["script", "style", "ix:header"];

/// Elements that may appear inside `<head>`. Any other start tag means the
/// author left `</head>` out and the body has begun.
const HEAD_CONTENT: &[&str] = &[
    "base", "basefont", "bgsound", "head", "link", "meta", "noframes", "noscript", "script",
    "style", "template", "title",
];

struct Tag {
    /// Offset just past the closing `>`, or the input length if unterminated.
    end: usize,
    /// Lowercased element name.
    name: String,
    closing: bool,
    self_closing: bool,
}

pub(super) fn scan(source: &str, builder: &mut Builder) {
    let bytes = source.as_bytes();
    let mut pos = text::start_offset(source, builder);
    let mut in_head = false;
    let mut in_head_noscript = false;

    while pos < bytes.len() {
        let text_end = next_markup(bytes, pos);
        if text_end > pos {
            if !in_head {
                text::scan_range(source, pos..text_end, builder, entities::decode);
            }
            pos = text_end;
            continue;
        }

        // `pos` is at a `<` that opens markup.
        match bytes.get(pos + 1) {
            Some(b'!') => {
                pos = if bytes[pos..].starts_with(b"<!--") {
                    comment_end(bytes, pos)
                } else {
                    find_byte(bytes, pos, b'>').map_or(bytes.len(), |at| at + 1)
                };
            }
            Some(b'?') => {
                pos = find_byte(bytes, pos, b'>').map_or(bytes.len(), |at| at + 1);
            }
            _ => {
                let tag = read_tag(source, pos);
                let tag_start = pos;
                pos = tag.end;
                let name = tag.name.as_str();

                if in_head {
                    if name == "noscript" {
                        in_head_noscript = !tag.closing;
                    }
                    let leaves_head = (tag.closing && name == "head")
                        || (!tag.closing && !in_head_noscript && !HEAD_CONTENT.contains(&name));
                    if leaves_head {
                        in_head = false;
                    }
                } else if !tag.closing && name == "head" {
                    in_head = true;
                }

                if !tag.closing && !tag.self_closing && DROPPED.contains(&name) {
                    pos = dropped_end(bytes, pos, name);
                    continue;
                }
                if !in_head && BLOCK.contains(&name) {
                    builder.separator(tag_start..tag.end, true);
                }
            }
        }
    }
}

/// Offset of the next `<` that opens markup, at or after `from`.
///
/// A `<` opens markup only when followed by a letter, `/` plus a letter, `!`
/// or `?`. Anything else (`a < b`, `<3`) is literal text.
fn next_markup(bytes: &[u8], from: usize) -> usize {
    let mut pos = from;
    while let Some(at) = find_byte(bytes, pos, b'<') {
        let opens = match bytes.get(at + 1) {
            Some(b'!' | b'?') => true,
            Some(b'/') => bytes.get(at + 2).is_some_and(u8::is_ascii_alphabetic),
            Some(byte) => byte.is_ascii_alphabetic(),
            None => false,
        };
        if opens {
            return at;
        }
        pos = at + 1;
    }
    bytes.len()
}

fn find_byte(bytes: &[u8], from: usize, needle: u8) -> Option<usize> {
    bytes
        .get(from..)?
        .iter()
        .position(|&byte| byte == needle)
        .map(|offset| from + offset)
}

/// Offset just past the `-->` closing the comment that starts at `pos`.
fn comment_end(bytes: &[u8], pos: usize) -> usize {
    let body = pos + 4;
    // `<!-->` and `<!--->` are complete, empty comments.
    if bytes.get(body) == Some(&b'>') {
        return body + 1;
    }
    if bytes.get(body..body + 2) == Some(b"->") {
        return body + 2;
    }
    bytes
        .get(body..)
        .and_then(|rest| rest.windows(3).position(|window| window == b"-->"))
        .map_or(bytes.len(), |offset| body + offset + 3)
}

/// Read the start or end tag beginning at `source[pos]`.
fn read_tag(source: &str, pos: usize) -> Tag {
    let bytes = source.as_bytes();
    let mut cursor = pos + 1;
    let closing = bytes.get(cursor) == Some(&b'/');
    if closing {
        cursor += 1;
    }
    let name_start = cursor;
    while bytes
        .get(cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
    {
        cursor += 1;
    }
    let name = source[name_start..cursor].to_ascii_lowercase();

    let mut self_closing = false;
    loop {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        match bytes.get(cursor) {
            None => break,
            Some(b'>') => {
                cursor += 1;
                break;
            }
            Some(b'/') => {
                self_closing = bytes.get(cursor + 1) == Some(&b'>');
                cursor += 1;
                continue;
            }
            Some(_) => {}
        }
        // Attribute name. A leading `=` belongs to the name, per the standard.
        cursor += 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>' | b'='))
        {
            cursor += 1;
        }
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        match bytes.get(cursor) {
            Some(&quote @ (b'"' | b'\'')) => {
                cursor = find_byte(bytes, cursor + 1, quote).map_or(bytes.len(), |at| at + 1);
            }
            _ => {
                while bytes
                    .get(cursor)
                    .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>')
                {
                    cursor += 1;
                }
            }
        }
    }

    Tag {
        end: cursor,
        name,
        closing,
        self_closing,
    }
}

/// Offset just past the end tag that closes the dropped element `name`, whose
/// content starts at `from`. Runs to the end of input if it is never closed.
fn dropped_end(bytes: &[u8], from: usize, name: &str) -> usize {
    let mut pos = from;
    while let Some(at) = find_byte(bytes, pos, b'<') {
        let candidate = at + 2;
        let matches_name = bytes.get(at + 1) == Some(&b'/')
            && bytes
                .get(candidate..candidate + name.len())
                .is_some_and(|slice| slice.eq_ignore_ascii_case(name.as_bytes()))
            && bytes
                .get(candidate + name.len())
                .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'));
        if matches_name {
            return find_byte(bytes, at, b'>').map_or(bytes.len(), |end| end + 1);
        }
        pos = at + 1;
    }
    bytes.len()
}
