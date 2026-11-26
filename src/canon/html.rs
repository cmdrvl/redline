//! HTML and XHTML text extraction that keeps byte positions.
//!
//! This is a scanner, not a tree builder. It finds markup boundaries the way
//! the WHATWG tokenizer does (quoted attribute values, comments, raw-text
//! elements) and hands everything else to the text scanner. A DOM parser
//! would be more forgiving of broken nesting, but it would also discard the
//! byte offsets this tool exists to preserve.

use super::entities;
use super::text;
use super::{Builder, Edges};

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

/// The scanner errs toward showing text. Where a browser would hide
/// something and the markup is too broken to be sure, the text is kept:
/// reporting a change in text nobody sees costs a glance, while hiding a
/// change in text somebody reads is the one failure this tool cannot have.
pub(super) fn scan(source: &str, builder: &mut Builder) {
    let bytes = source.as_bytes();
    let mut pos = text::start_offset(source, builder);
    // Inside the document head: from a `<head>` start tag that comes before
    // any body content, until something ends the head.
    let mut in_head = false;
    // Body content has been seen, so a later `<head>` tag is ignored, as a
    // browser ignores it. A fragment is cut from a body by construction.
    let mut body_started = builder.edges == Edges::Fragment;
    // Where the last search for an end tag landed, one per kind of search.
    let mut ix_header_ends = Lookahead::default();
    let mut title_ends = Lookahead::default();
    let mut head_title_ends = Lookahead::default();

    while pos < bytes.len() {
        let text_end = next_markup(bytes, pos);
        if text_end > pos {
            // Text inside the head implicitly ends it and is rendered.
            if in_head && !source[pos..text_end].chars().all(char::is_whitespace) {
                in_head = false;
            }
            if !in_head {
                body_started |= !source[pos..text_end].chars().all(char::is_whitespace);
                text::scan_range(source, pos..text_end, builder, entities::decode);
            }
            pos = text_end;
            continue;
        }

        // `pos` is at a `<` that opens markup.
        match bytes.get(pos + 1) {
            Some(b'!') => {
                if bytes[pos..].starts_with(b"<!--") {
                    pos = comment_end(bytes, pos);
                } else if bytes[pos..].starts_with(CDATA_OPEN) {
                    // XHTML shows CDATA content; an HTML parser hides it.
                    // Filings are often XHTML, so it is kept as literal text.
                    let content = pos + CDATA_OPEN.len();
                    let close = find_bytes(bytes, content, b"]]>");
                    let content_end = close.unwrap_or(bytes.len());
                    in_head = false;
                    body_started = true;
                    text::scan_range(source, content..content_end, builder, no_references);
                    pos = close.map_or(bytes.len(), |at| at + 3);
                } else if let Some(at) = find_byte(bytes, pos, b'>') {
                    pos = at + 1;
                } else {
                    keep_unclosed(source, pos, builder);
                    pos = bytes.len();
                }
            }
            // A processing instruction, or `</` not followed by a letter:
            // nothing up to the next `>` is rendered (`</>` is simply
            // dropped).
            Some(b'?') => match find_byte(bytes, pos, b'>') {
                Some(at) => pos = at + 1,
                None => {
                    keep_unclosed(source, pos, builder);
                    pos = bytes.len();
                }
            },
            Some(b'/') if !bytes.get(pos + 2).is_some_and(u8::is_ascii_alphabetic) => {
                match find_byte(bytes, pos, b'>') {
                    Some(at) => pos = at + 1,
                    None => {
                        keep_unclosed(source, pos, builder);
                        pos = bytes.len();
                    }
                }
            }
            _ => {
                let tag = read_tag(source, pos);
                let tag_start = pos;
                pos = tag.end;
                let name = tag.name.as_str();

                if in_head {
                    let leaves_head = (tag.closing && name == "head")
                        || (!tag.closing && !HEAD_CONTENT.contains(&name));
                    if leaves_head {
                        in_head = false;
                        body_started = true;
                    }
                } else if !tag.closing && name == "head" && !body_started {
                    in_head = true;
                } else if !tag.closing && name != "html" && !HEAD_CONTENT.contains(&name) {
                    body_started = true;
                }

                if !in_head && BLOCK.contains(&name) {
                    builder.separator(tag_start..tag.end, true);
                }
                if tag.closing {
                    continue;
                }

                match name {
                    // Code. Never rendered; an unclosed one runs to the end.
                    // `<script src="a.js"/>` is complete in XHTML, which is
                    // what filings are, so a self-closed one hides nothing.
                    // An HTML parser would hide what follows it; showing
                    // that text is the safe side to be wrong on.
                    "script" | "style" if !tag.self_closing => {
                        pos = end_tag(bytes, pos, name).map_or(bytes.len(), |(_, after)| after);
                    }
                    // The hidden Inline XBRL block. Unlike script and style
                    // it is an ordinary element to an HTML parser, so it is
                    // dropped only when its end tag is really there.
                    "ix:header" if !tag.self_closing => {
                        if let Some((_, after)) =
                            ix_header_ends.find(pos, |from| end_tag(bytes, from, name))
                        {
                            pos = after;
                        }
                    }
                    // Content is text with character references; markup in
                    // it is literal. The document title is not rendered on
                    // the page, so a title in the head is dropped.
                    // A title that is self-closed or never closed is treated
                    // as an ordinary tag, so it cannot hide what follows. In
                    // the head, "never closed" includes reaching `</head>` or
                    // `<body>` first: a later `</title>`, such as an inline
                    // SVG's, must not make the body part of the title.
                    "title" if !tag.self_closing => {
                        let end = if in_head {
                            head_title_ends
                                .find(pos, |from| head_title_end(bytes, from))
                                .filter(|&(at, _)| is_end_tag(bytes, at, "title"))
                        } else {
                            title_ends.find(pos, |from| end_tag(bytes, from, name))
                        };
                        if let Some((content_end, after)) = end {
                            if !in_head {
                                let content = pos..content_end;
                                text::scan_range(source, content, builder, entities::decode);
                            }
                            pos = after;
                        }
                    }
                    "textarea" => {
                        let (content_end, after) =
                            end_tag(bytes, pos, name).unwrap_or((bytes.len(), bytes.len()));
                        if !in_head {
                            text::scan_range(source, pos..content_end, builder, entities::decode);
                        }
                        pos = after;
                    }
                    // Content is literal text, references and all.
                    "xmp" => {
                        let (content_end, after) =
                            end_tag(bytes, pos, name).unwrap_or((bytes.len(), bytes.len()));
                        builder.separator(tag_start..tag.end, true);
                        text::scan_range(source, pos..content_end, builder, no_references);
                        if after > content_end {
                            builder.separator(content_end..after, true);
                        }
                        pos = after;
                    }
                    // Everything after `<plaintext>` is literal text.
                    "plaintext" => {
                        builder.separator(tag_start..tag.end, true);
                        text::scan_range(source, pos..bytes.len(), builder, no_references);
                        pos = bytes.len();
                    }
                    _ => {}
                }
            }
        }
    }
}

const CDATA_OPEN: &[u8] = b"<![CDATA[";

/// For text in which `&` is an ordinary character.
fn no_references(_: &str, _: usize, _: usize) -> Option<(String, usize)> {
    None
}

/// Offset of the next `<` that opens markup, at or after `from`.
///
/// A `<` opens markup only when followed by a letter, `/`, `!` or `?`.
/// Anything else (`a < b`, `<3`) is literal text.
fn next_markup(bytes: &[u8], from: usize) -> usize {
    let mut pos = from;
    while let Some(at) = find_byte(bytes, pos, b'<') {
        let opens = match bytes.get(at + 1) {
            Some(b'!' | b'?') => true,
            Some(b'/') => bytes.get(at + 2).is_some(),
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

fn find_bytes(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// Offset just past the end of the comment that starts at `pos`. A comment
/// ends at `-->` or at `--!>`, whichever comes first.
fn comment_end(bytes: &[u8], pos: usize) -> usize {
    let body = pos + 4;
    // `<!-->` and `<!--->` are complete, empty comments.
    if bytes.get(body) == Some(&b'>') {
        return body + 1;
    }
    if bytes.get(body..body + 2) == Some(b"->") {
        return body + 2;
    }
    let mut at = body;
    while let Some(dashes) = find_bytes(bytes, at, b"--") {
        match bytes.get(dashes + 2..) {
            Some([b'>', ..]) => return dashes + 3,
            Some([b'!', b'>', ..]) => return dashes + 4,
            _ => at = dashes + 1,
        }
    }
    bytes.len()
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

/// Markup that opens at `pos` and is never closed. A browser hides everything
/// after it. It is broken, so the text is kept and diffed.
fn keep_unclosed(source: &str, pos: usize, builder: &mut Builder) {
    text::scan_range(source, pos..source.len(), builder, entities::decode);
}

/// Remembers where a forward search landed. The scanner only moves forward, so
/// a later search that starts at or before that place must land there too.
/// Without this, a document that repeats an unclosed tag thousands of times
/// searches to the end of the input once per tag.
#[derive(Default)]
struct Lookahead {
    last: Option<Option<(usize, usize)>>,
}

impl Lookahead {
    /// `search(from)` returns the first match at or after `from`.
    fn find(
        &mut self,
        from: usize,
        search: impl FnOnce(usize) -> Option<(usize, usize)>,
    ) -> Option<(usize, usize)> {
        match self.last {
            Some(None) => None,
            Some(Some((at, after))) if from <= at => Some((at, after)),
            _ => {
                let found = search(from);
                self.last = Some(found);
                found
            }
        }
    }
}

/// True when the `<` at `at` starts the tag `</name`.
fn is_end_tag(bytes: &[u8], at: usize, name: &str) -> bool {
    bytes.get(at + 1) == Some(&b'/') && names_tag(bytes, at + 2, name)
}

/// True when the tag name at `start` is `name`.
fn names_tag(bytes: &[u8], start: usize, name: &str) -> bool {
    bytes
        .get(start..start + name.len())
        .is_some_and(|slice| slice.eq_ignore_ascii_case(name.as_bytes()))
        && bytes
            .get(start + name.len())
            .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

/// The first tag at or after `from` that matches, as the offset where it
/// starts and the offset just past it.
fn find_tag(bytes: &[u8], from: usize, matches: impl Fn(usize) -> bool) -> Option<(usize, usize)> {
    let mut pos = from;
    while let Some(at) = find_byte(bytes, pos, b'<') {
        if matches(at) {
            let after = find_byte(bytes, at, b'>').map_or(bytes.len(), |end| end + 1);
            return Some((at, after));
        }
        pos = at + 1;
    }
    None
}

/// The end tag that closes the raw-content element `name`, whose content
/// starts at `from`. `None` if the element is never closed.
fn end_tag(bytes: &[u8], from: usize, name: &str) -> Option<(usize, usize)> {
    find_tag(bytes, from, |at| is_end_tag(bytes, at, name))
}

/// What comes first after a title in the head: its end tag, or a sign that
/// the head is over and the title was never closed.
fn head_title_end(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    find_tag(bytes, from, |at| {
        is_end_tag(bytes, at, "title")
            || is_end_tag(bytes, at, "head")
            || names_tag(bytes, at + 1, "body")
    })
}
