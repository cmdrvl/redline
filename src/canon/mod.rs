//! Canonicalization with a raw byte source map.
//!
//! Each mode turns the raw input into normalized text and records, for every
//! canonical byte, the exact raw byte range it came from. Nothing downstream
//! ever reports a canonical offset: hunks are projected back through this map.

mod entities;
mod html;
mod text;

use std::ops::Range;

use serde::Serialize;

/// How an input is normalized before diffing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Mode {
    /// Byte identity. Every byte difference is a change.
    #[serde(rename = "raw")]
    Raw,
    /// Runs of Unicode whitespace collapse to one space; edges are trimmed.
    #[serde(rename = "whitespace")]
    Whitespace,
    /// Markup is dropped, entities are decoded, whitespace collapses.
    #[serde(rename = "html-text")]
    HtmlText,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Raw => "raw",
            Mode::Whitespace => "whitespace",
            Mode::HtmlText => "html-text",
        }
    }
}

/// Whether the edges of the input are trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edges {
    /// A whole document: leading and trailing separators are dropped.
    Document,
    /// A slice cut out of a document: a leading or trailing separator is kept
    /// as one space, exactly as it appears inside the parent document.
    Fragment,
}

/// One run of canonical text and the raw bytes it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    canon_start: usize,
    canon_len: usize,
    raw_start: usize,
    raw_len: usize,
    /// Canonical bytes equal raw bytes one for one, so offsets inside the
    /// segment map linearly. Otherwise the segment is an atom (a decoded
    /// entity, a collapsed separator) and maps to its whole raw range.
    verbatim: bool,
}

/// Normalized text plus the map back to raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalDoc {
    /// Normalized UTF-8 text. This is what gets diffed.
    pub text: String,
    segments: Vec<Segment>,
    /// Canonical offsets of separator spaces that stand for a block boundary
    /// (a blank line, a block-level tag). Ascending.
    pub block_breaks: Vec<usize>,
    /// Length of the raw input in bytes.
    pub raw_len: usize,
}

impl CanonicalDoc {
    /// Raw byte span `[start, end)` covering the canonical range `[start, end)`.
    ///
    /// Both ends must sit on character boundaries of `text`. An empty range
    /// maps to an empty raw span at the position of `range.start`.
    pub fn raw_span(&self, range: Range<usize>) -> Range<usize> {
        if range.start >= range.end {
            let at = self.raw_start_of(range.start);
            return at..at;
        }
        let start = self.raw_start_of(range.start);
        let end = self.raw_end_of(range.end);
        start..end.max(start)
    }

    fn segment_at(&self, canon: usize) -> Option<&Segment> {
        let idx = self
            .segments
            .partition_point(|seg| seg.canon_start + seg.canon_len <= canon);
        self.segments.get(idx)
    }

    /// Raw offset where the canonical character starting at `canon` begins.
    fn raw_start_of(&self, canon: usize) -> usize {
        match self.segment_at(canon) {
            Some(seg) if seg.verbatim => seg.raw_start + (canon - seg.canon_start),
            Some(seg) => seg.raw_start,
            None => self
                .segments
                .last()
                .map_or(0, |seg| seg.raw_start + seg.raw_len),
        }
    }

    /// Raw offset just past the canonical character ending at `canon`.
    fn raw_end_of(&self, canon: usize) -> usize {
        debug_assert!(canon > 0);
        match self.segment_at(canon - 1) {
            Some(seg) if seg.verbatim => seg.raw_start + (canon - seg.canon_start),
            Some(seg) => seg.raw_start + seg.raw_len,
            None => self.raw_len,
        }
    }
}

/// Accumulates canonical text and its source map.
struct Builder {
    text: String,
    segments: Vec<Segment>,
    block_breaks: Vec<usize>,
    /// Raw range of a separator seen since the last text, not yet emitted.
    pending: Option<Range<usize>>,
    pending_block: bool,
    edges: Edges,
}

impl Builder {
    fn new(edges: Edges, capacity: usize) -> Self {
        Builder {
            text: String::with_capacity(capacity),
            segments: Vec::new(),
            block_breaks: Vec::new(),
            pending: None,
            pending_block: false,
            edges,
        }
    }

    /// Record whitespace or a block boundary at `raw`. Adjacent separators
    /// merge into one space.
    fn separator(&mut self, raw: Range<usize>, block: bool) {
        match &mut self.pending {
            Some(range) => range.end = raw.end,
            None => self.pending = Some(raw),
        }
        self.pending_block |= block;
    }

    fn flush_separator(&mut self) {
        let Some(raw) = self.pending.take() else {
            return;
        };
        let block = std::mem::take(&mut self.pending_block);
        if self.text.is_empty() && self.edges == Edges::Document {
            return;
        }
        if block {
            self.block_breaks.push(self.text.len());
        }
        self.push_segment(" ", raw, false);
    }

    /// Text copied byte for byte from `raw`.
    fn verbatim(&mut self, source: &str, raw: Range<usize>) {
        if raw.is_empty() {
            return;
        }
        self.flush_separator();
        let piece = &source[raw.clone()];
        if let Some(last) = self.segments.last_mut()
            && last.verbatim
            && last.raw_start + last.raw_len == raw.start
        {
            last.canon_len += piece.len();
            last.raw_len += piece.len();
            self.text.push_str(piece);
            return;
        }
        self.push_segment(piece, raw, true);
    }

    /// Text that stands for `raw` without equalling it (a decoded entity).
    fn mapped(&mut self, decoded: &str, raw: Range<usize>) {
        if decoded.is_empty() {
            return;
        }
        self.flush_separator();
        self.push_segment(decoded, raw, false);
    }

    fn push_segment(&mut self, piece: &str, raw: Range<usize>, verbatim: bool) {
        self.segments.push(Segment {
            canon_start: self.text.len(),
            canon_len: piece.len(),
            raw_start: raw.start,
            raw_len: raw.end - raw.start,
            verbatim,
        });
        self.text.push_str(piece);
    }

    fn finish(mut self, raw_len: usize) -> CanonicalDoc {
        if self.edges == Edges::Fragment {
            self.flush_separator();
        }
        CanonicalDoc {
            text: self.text,
            segments: self.segments,
            block_breaks: self.block_breaks,
            raw_len,
        }
    }
}

/// Canonicalize `source` under `mode`.
///
/// `source` is the raw input, already validated as UTF-8. All spans in the
/// result index into it.
pub fn canonicalize(source: &str, mode: Mode, edges: Edges) -> CanonicalDoc {
    match mode {
        Mode::Raw => raw_identity(source),
        Mode::Whitespace => {
            let mut builder = Builder::new(edges, source.len());
            text::scan(source, &mut builder);
            builder.finish(source.len())
        }
        Mode::HtmlText => {
            let mut builder = Builder::new(edges, source.len() / 2);
            html::scan(source, &mut builder);
            builder.finish(source.len())
        }
    }
}

fn raw_identity(source: &str) -> CanonicalDoc {
    let segments = if source.is_empty() {
        Vec::new()
    } else {
        vec![Segment {
            canon_start: 0,
            canon_len: source.len(),
            raw_start: 0,
            raw_len: source.len(),
            verbatim: true,
        }]
    };
    CanonicalDoc {
        text: source.to_owned(),
        segments,
        block_breaks: Vec::new(),
        raw_len: source.len(),
    }
}

/// What an input looks like from the way it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Opens with `<!DOCTYPE html` or an `<html>` tag: an HTML document.
    Html,
    /// Opens with text.
    Text,
    /// Opens with some other tag. It may be an HTML fragment, in which case
    /// tags should be ignored, or Markdown, XML or old tagged EDGAR text, in
    /// which case what sits in angle brackets is content. Guessing wrong one
    /// way hides real changes, so the caller is asked.
    Unclear,
}

/// The lines EDGAR wraps around each document of a filing. A document saved
/// from EDGAR often still carries them in front of its HTML.
const EDGAR_WRAPPER: &[&str] = &[
    "<document>",
    "<type>",
    "<sequence>",
    "<filename>",
    "<description>",
];

/// Classify an input by how it opens, after an optional byte order mark, XML
/// declaration, comments and EDGAR wrapper lines.
///
/// Mentioning a tag is not enough to be HTML. Plain text that talks about
/// `<head>` or `Vec<String>` must not be read as markup, because everything
/// in angle brackets would then be hidden from the diff.
pub fn detect_format(source: &str) -> Format {
    let bytes = source.as_bytes();
    let mut pos = if source.starts_with('\u{feff}') { 3 } else { 0 };
    loop {
        while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
            pos += 1;
        }
        let rest = &bytes[pos..];
        let opens_with = |tag: &[u8]| {
            rest.get(..tag.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(tag))
        };
        let close: &[u8] = if rest.starts_with(b"<!--") {
            b"-->"
        } else if rest.starts_with(b"<?") {
            b"?>"
        } else if opens_with(b"<text>") {
            // The document starts right after this tag.
            b">"
        } else if EDGAR_WRAPPER.iter().any(|tag| opens_with(tag.as_bytes())) {
            // A wrapper line with its value, such as `<TYPE>10-K`.
            b"\n"
        } else {
            break;
        };
        match rest.windows(close.len()).position(|window| window == close) {
            Some(at) => pos += at + close.len(),
            None => return Format::Text,
        }
    }
    let rest = &bytes[pos..];
    let opens_with = |tag: &[u8]| {
        rest.get(..tag.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(tag))
    };
    let html_tag = opens_with(b"<html")
        && rest
            .get(5)
            .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/'));
    if opens_with(b"<!doctype html") || html_tag {
        Format::Html
    } else if rest.first() == Some(&b'<') && rest.get(1).is_some_and(u8::is_ascii_alphabetic) {
        Format::Unclear
    } else {
        Format::Text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(source: &str, mode: Mode) -> CanonicalDoc {
        canonicalize(source, mode, Edges::Document)
    }

    #[test]
    fn raw_is_identity() {
        let d = doc("a  b\r\n", Mode::Raw);
        assert_eq!(d.text, "a  b\r\n");
        assert_eq!(d.raw_span(1..3), 1..3);
    }

    #[test]
    fn whitespace_collapses_and_trims() {
        let d = doc("  alpha \t beta\r\ngamma  \n", Mode::Whitespace);
        assert_eq!(d.text, "alpha beta gamma");
        assert_eq!(d.raw_span(0..5), 2..7);
        // The single canonical space maps to the whole raw run.
        assert_eq!(d.raw_span(5..6), 7..10);
        assert_eq!(d.raw_span(6..10), 10..14);
        assert!(d.block_breaks.is_empty());
    }

    #[test]
    fn blank_line_is_a_block_break() {
        let d = doc("one\n\ntwo\nthree", Mode::Whitespace);
        assert_eq!(d.text, "one two three");
        assert_eq!(d.block_breaks, vec![3]);
    }

    #[test]
    fn fragment_keeps_edge_separators() {
        let d = canonicalize(" \n x ", Mode::Whitespace, Edges::Fragment);
        assert_eq!(d.text, " x ");
    }

    #[test]
    fn byte_order_mark_is_not_text() {
        let d = doc("\u{feff}hello", Mode::Whitespace);
        assert_eq!(d.text, "hello");
        assert_eq!(d.raw_span(0..5), 3..8);
    }

    #[test]
    fn html_drops_markup_and_decodes_entities() {
        let src = "<p>Tom &amp; Jerry&#8217;s&nbsp;show</p>";
        let d = doc(src, Mode::HtmlText);
        assert_eq!(d.text, "Tom & Jerry\u{2019}s show");
        let amp = d.text.find('&').unwrap();
        assert_eq!(&src[d.raw_span(amp..amp + 1)], "&amp;");
        let quote = d.text.find('\u{2019}').unwrap();
        assert_eq!(&src[d.raw_span(quote..quote + 3)], "&#8217;");
        let space = d.text.rfind(' ').unwrap();
        assert_eq!(&src[d.raw_span(space..space + 1)], "&nbsp;");
    }

    #[test]
    fn html_word_split_by_inline_tag_keeps_one_span() {
        let src = "<p>foo<b>bar</b> baz</p>";
        let d = doc(src, Mode::HtmlText);
        assert_eq!(d.text, "foobar baz");
        assert_eq!(&src[d.raw_span(0..6)], "foo<b>bar");
    }

    #[test]
    fn html_block_tags_separate_text() {
        let src = "<table><tr><td>a</td><td>b</td></tr></table><div>c</div>d<br>e";
        let d = doc(src, Mode::HtmlText);
        assert_eq!(d.text, "a b c d e");
        assert_eq!(d.block_breaks.len(), 4);
    }

    #[test]
    fn html_skips_head_script_style_comments() {
        let src = "<html><head><title>T</title><style>p{x:1}</style></head>\
                   <body><!-- note --><script>var a = '<p>no</p>';</script>\
                   <p>yes</p></body></html>";
        assert_eq!(doc(src, Mode::HtmlText).text, "yes");
    }

    #[test]
    fn html_head_without_close_ends_at_first_body_element() {
        let src = "<head><meta charset=utf-8><title>T</title><p>shown";
        assert_eq!(doc(src, Mode::HtmlText).text, "shown");
    }

    #[test]
    fn html_skips_inline_xbrl_header() {
        let src = "<body><div style=\"display:none\"><ix:header><ix:hidden>\
                   <ix:nonNumeric name=\"dei:X\">0001</ix:nonNumeric></ix:hidden>\
                   </ix:header></div><p>Revenue <ix:nonFraction>100</ix:nonFraction></p>";
        assert_eq!(doc(src, Mode::HtmlText).text, "Revenue 100");
    }

    #[test]
    fn html_quoted_attribute_may_hold_angle_bracket() {
        let src = "<a title=\"a > b\" href='x>y'>link</a> <span data-x=it's>t</span>";
        assert_eq!(doc(src, Mode::HtmlText).text, "link t");
    }

    #[test]
    fn html_stray_angle_and_ampersand_are_text() {
        let src = "<p>1 < 2 & 3 &bogus; &amp</p>";
        assert_eq!(doc(src, Mode::HtmlText).text, "1 < 2 & 3 &bogus; &");
    }

    #[test]
    fn html_numeric_references_follow_the_standard() {
        let src = "&#x41;&#65;&#150;&#0;&#xD800;";
        assert_eq!(doc(src, Mode::HtmlText).text, "AA\u{2013}\u{fffd}\u{fffd}");
    }

    #[test]
    fn html_documents_are_detected() {
        for source in [
            "<!DOCTYPE HTML><p>x",
            "<?xml version='1.0'?>\n<html xmlns=\"x\">",
            "\u{feff}\n<!-- c -->\n<HTML>\n<body>",
            "<DOCUMENT>\n<TYPE>10-K\n<SEQUENCE>1\n<FILENAME>a.htm\n<TEXT>\n<!DOCTYPE html PUBLIC \"x\">",
            "<DOCUMENT>\n<TYPE>8-K\n<TEXT><html><body>",
        ] {
            assert_eq!(detect_format(source), Format::Html, "{source:?}");
        }
    }

    #[test]
    fn text_that_mentions_tags_is_text() {
        for source in [
            "plain text with a < b",
            "Notes: the <head> block.\nPayment is $100.\n",
            "Notes on <html> docs.\nUse Vec<String> here.\n",
            "<DOCUMENT>\n<TYPE>8-K\n<TEXT>\nPlain text press release\n",
            "<!-- never closed <html>",
            "< not a tag",
        ] {
            assert_eq!(detect_format(source), Format::Text, "{source:?}");
        }
    }

    /// Each of these was once read as HTML, hiding what sits in angle
    /// brackets. None can be told from an HTML fragment by its opening.
    #[test]
    fn a_leading_tag_that_is_not_html_is_unclear() {
        for source in [
            "<div class=\"a\"><p>x</p></div>",
            "<h1 align=\"center\">Tool</h1>\n\nUse `map<K, V>` here.\n",
            "<b>Note</b> see vector<int> x",
            "<i> is less than <j> in the loop",
            "<DOCUMENT>\n<TYPE>EX-10\n<TEXT>\n<TABLE>\n<S> <C>\nSee note <F1>.\n",
            "<TYPE>writer notes\n<p>\nList<String>",
            "<script>let x = 1;</script>",
            "<htmlish>",
        ] {
            assert_eq!(detect_format(source), Format::Unclear, "{source:?}");
        }
    }

    // The cases below were each a missed change: the scanner hid text that a
    // browser renders, so an edit to it was reported as formatting only.

    fn html(source: &str) -> String {
        doc(source, Mode::HtmlText).text
    }

    #[test]
    fn html_text_in_head_is_rendered() {
        assert_eq!(html("<html><head>Visible text"), "Visible text");
        assert_eq!(
            html("<head><meta charset=utf-8>Price 100 dollars</html>"),
            "Price 100 dollars"
        );
        assert_eq!(html("<head></p>alpha</div>more"), "alpha more");
        assert_eq!(html("<head><noscript><p>alpha</p>"), "alpha");
        assert_eq!(html("<head lang=en>shown"), "shown");
    }

    #[test]
    fn html_head_tag_after_body_content_is_ignored() {
        assert_eq!(
            html("<body><p>intro</p><head>Rate is 5%<p>x</p></body>"),
            "intro Rate is 5% x"
        );
        assert_eq!(html("<html>pre<head>OLD<body>z"), "preOLD z");
        assert_eq!(
            html("<p>a</p><head><title>T</title>b"),
            "a Tb",
            "a title outside the document head is shown"
        );
    }

    #[test]
    fn html_comment_closed_by_bang_ends_there() {
        assert_eq!(
            html("<p>a</p><!-- x --!>VISIBLE<!-- y --><p>b</p>"),
            "a VISIBLE b"
        );
        assert_eq!(html("<p>a<!----!>one<!-- -->"), "aone");
        assert_eq!(html("a<!-- - -- x -->b"), "ab");
        assert_eq!(html("a<!-- never closed"), "a");
    }

    #[test]
    fn html_markup_inside_text_elements_is_literal() {
        assert_eq!(
            html("<body><textarea><b>x</b> &amp; y</textarea>z"),
            "<b>x</b> & yz"
        );
        assert_eq!(html("<body><xmp><i>x</i> &amp;</xmp>z"), "<i>x</i> &amp; z");
        assert_eq!(
            html("<p>a</p><plaintext><b>x</b></plaintext>"),
            "a <b>x</b></plaintext>"
        );
        assert_eq!(html("<body><textarea><!-- old -->"), "<!-- old -->");
    }

    #[test]
    fn html_unclosed_xbrl_header_does_not_swallow_the_document() {
        assert_eq!(html("<body><ix:header>a</ix:hidden>OLD text"), "aOLD text");
        assert_eq!(html("<body><ix:header a=b/>OLD text"), "OLD text");
        assert_eq!(html("<body><ix:header/>shown"), "shown");
    }

    #[test]
    fn html_script_and_style_hide_to_the_end_when_unclosed() {
        assert_eq!(html("a<script>var x = 1;"), "a");
        assert_eq!(html("a<STYLE>p{}</Style >b"), "ab");
    }

    /// In XHTML `<script src="a.js"/>` and `<title/>` are complete elements.
    /// Reading them the HTML way hid the whole rest of the document.
    #[test]
    fn html_self_closed_script_style_and_title_hide_nothing() {
        let page = |head: &str| {
            format!(
                "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\">\
                 <head>{head}</head><body><p>Pay 100 now</p></body></html>"
            )
        };
        for head in [
            "<title/>",
            "<script src=\"a.js\"/>",
            "<style/>",
            "<title />",
        ] {
            assert_eq!(html(&page(head)), "Pay 100 now", "{head}");
        }
        // The price of that reading: in plain HTML the script body shows.
        assert_eq!(html("a<script/>shown</script>b"), "ashownb");
    }

    #[test]
    fn html_unclosed_title_hides_nothing() {
        assert_eq!(
            html("<head><title>Doc</titel></head><body><p>Pay 100</p>"),
            "Doc Pay 100"
        );
    }

    #[test]
    fn html_bogus_end_tag_with_no_close_keeps_the_text() {
        assert_eq!(
            html("a </ b and then visible text"),
            "a </ b and then visible text"
        );
        assert_eq!(html("Ratio is </3 OLD"), "Ratio is </3 OLD");
    }

    #[test]
    fn html_cdata_content_is_text() {
        assert_eq!(
            html("<p>a <![CDATA[x < y &amp;]]> b</p>"),
            "a x < y &amp; b"
        );
        assert_eq!(html("<p>a<![CDATA[open"), "aopen");
    }

    #[test]
    fn html_bogus_end_tags_are_dropped() {
        assert_eq!(html("a</>b</ x>c</3 >d"), "abcd");
        assert_eq!(html("a</"), "a</");
    }
}
