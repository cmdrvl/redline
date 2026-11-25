//! End-to-end comparison: validate, canonicalize, diff, classify, report.

use std::ops::Range;

use serde_json::json;

use crate::canon::{CanonicalDoc, Edges, Mode, canonicalize, looks_like_html};
use crate::classify::{HunkClass, classify};
use crate::diff::{Change, diff};
use crate::refusal::{Refusal, RefusalCode};
use crate::report::{Hunk, InputInfo, Inputs, Outcome, REPORT_VERSION, Report, Summary, TOOL};
use crate::tokens::{Interner, TokenDoc, tokenize};

/// Explicit-mode excerpts are cut to this many bytes of canonical text.
pub const EXCERPT_MAX_BYTES: usize = 2000;

/// Modes named by the design that this release does not implement.
const PLANNED_MODES: &[&str] = &["markdown", "xml-text", "docx"];

/// Domain-separation string for the per-input hunk digest key.
const HUNK_KEY_CONTEXT: &str = "redline.report.v1 hunk digest key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChoice {
    /// Pick `html-text` or `whitespace` from the content of each input.
    Auto,
    Fixed(Mode),
}

impl ModeChoice {
    /// Parse a `--mode` value.
    pub fn parse(value: &str) -> Result<Self, Refusal> {
        match value {
            "auto" => Ok(ModeChoice::Auto),
            "raw" => Ok(ModeChoice::Fixed(Mode::Raw)),
            "whitespace" => Ok(ModeChoice::Fixed(Mode::Whitespace)),
            "html-text" => Ok(ModeChoice::Fixed(Mode::HtmlText)),
            other => Err(Refusal::new(
                RefusalCode::ModeUnsupported,
                json!({
                    "mode": other,
                    "available": ["auto", "raw", "whitespace", "html-text"],
                    "planned": PLANNED_MODES.contains(&other),
                }),
                None,
            )),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub mode: ModeChoice,
    /// Include canonical text excerpts for semantic hunks.
    pub explicit: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            mode: ModeChoice::Auto,
            explicit: false,
        }
    }
}

/// One input document.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub bytes: &'a [u8],
    /// Shown in the report and in refusal hints. Not read by the library.
    pub path: Option<&'a str>,
}

impl Input<'_> {
    fn info(&self) -> InputInfo {
        InputInfo {
            path: self.path.map(str::to_owned),
            bytes: Some(self.bytes.len() as u64),
            digest: Some(format!("blake3:{}", blake3::hash(self.bytes).to_hex())),
        }
    }

    fn label(&self, fallback: &str) -> String {
        self.path.unwrap_or(fallback).to_owned()
    }
}

/// Keyed digest of a hunk's raw bytes.
///
/// The key is derived from the whole input the bytes came from. Anyone
/// holding that input can recompute the digest and confirm the span. Anyone
/// without it cannot test guesses against the digest, which a plain hash of a
/// short hunk (`5.25%`) would allow.
pub fn hunk_digest(input: &[u8], span: Range<usize>) -> String {
    let key = blake3::derive_key(HUNK_KEY_CONTEXT, input);
    format!(
        "blake3-keyed:{}",
        blake3::keyed_hash(&key, &input[span]).to_hex()
    )
}

/// Compare two documents.
///
/// `Err` is a refusal: no verdict could be produced. Wrap it with
/// [`Report::refused`] to emit it in the standard envelope.
pub fn compare(old: Input<'_>, new: Input<'_>, options: &Options) -> Result<Report, Refusal> {
    let old_text = validate(&old, "old")?;
    let new_text = validate(&new, "new")?;
    let mode = resolve_mode(options.mode, old_text, new_text, &old, &new)?;

    let inputs = Inputs {
        old: old.info(),
        new: new.info(),
    };

    let old_doc = canonicalize(old_text, mode, Edges::Document);
    let new_doc = canonicalize(new_text, mode, Edges::Document);
    let mut interner = Interner::default();
    let old_tokens = tokenize(&old_doc, mode, &mut interner);
    let new_tokens = tokenize(&new_doc, mode, &mut interner);

    // Diff algorithms are not symmetric: A against B can align differently
    // from B against A. Always diff in one fixed order and mirror the result,
    // so swapping the inputs swaps the report and nothing else.
    let changes = if old.bytes == new.bytes {
        Vec::new()
    } else if old.bytes <= new.bytes {
        diff(&old_tokens, &new_tokens)
    } else {
        diff(&new_tokens, &old_tokens)
            .into_iter()
            .map(|change| Change {
                old: change.new,
                new: change.old,
            })
            .collect()
    };

    let sides = Sides {
        old: Side {
            raw: old.bytes,
            doc: &old_doc,
            tokens: &old_tokens,
            key: blake3::derive_key(HUNK_KEY_CONTEXT, old.bytes),
        },
        new: Side {
            raw: new.bytes,
            doc: &new_doc,
            tokens: &new_tokens,
            key: blake3::derive_key(HUNK_KEY_CONTEXT, new.bytes),
        },
        explicit: options.explicit,
    };
    let hunks = if old.bytes == new.bytes {
        Vec::new()
    } else {
        build_hunks(&sides, &changes)
    };

    let summary = summarize(&hunks, &old_tokens, &new_tokens);
    let outcome = if summary.semantic_hunks == 0 {
        Outcome::NoSemanticChange
    } else {
        Outcome::ChangesFound
    };
    Ok(Report {
        version: REPORT_VERSION,
        tool: TOOL,
        outcome,
        mode: Some(mode),
        inputs,
        summary: Some(summary),
        hunks,
        refusal: None,
    })
}

/// Check that an input is non-empty UTF-8 text.
fn validate<'a>(input: &Input<'a>, side: &str) -> Result<&'a str, Refusal> {
    if input.bytes.is_empty() {
        return Err(Refusal::new(
            RefusalCode::EmptyInput,
            json!({ "input": side, "path": input.path }),
            None,
        ));
    }
    let extract_hint = || {
        let path = input.label(&format!("<{side}>"));
        if input.bytes.starts_with(b"%PDF") {
            Some(format!("pdftotext -layout {path} {path}.txt"))
        } else {
            None
        }
    };
    if let Some(at) = input.bytes.iter().position(|&byte| byte == 0) {
        return Err(Refusal::new(
            RefusalCode::BinaryInput,
            json!({ "input": side, "path": input.path, "reason": "nul_byte", "offset": at }),
            extract_hint(),
        ));
    }
    std::str::from_utf8(input.bytes).map_err(|error| {
        Refusal::new(
            RefusalCode::BinaryInput,
            json!({
                "input": side,
                "path": input.path,
                "reason": "invalid_utf8",
                "offset": error.valid_up_to(),
            }),
            extract_hint(),
        )
    })
}

fn resolve_mode(
    choice: ModeChoice,
    old_text: &str,
    new_text: &str,
    old: &Input<'_>,
    new: &Input<'_>,
) -> Result<Mode, Refusal> {
    let detect = |text: &str| {
        if looks_like_html(text) {
            Mode::HtmlText
        } else {
            Mode::Whitespace
        }
    };
    match choice {
        ModeChoice::Fixed(mode) => Ok(mode),
        ModeChoice::Auto => {
            let old_mode = detect(old_text);
            let new_mode = detect(new_text);
            if old_mode == new_mode {
                return Ok(old_mode);
            }
            Err(Refusal::new(
                RefusalCode::ModeAmbiguous,
                json!({ "old": old_mode.as_str(), "new": new_mode.as_str() }),
                Some(format!(
                    "redline {} {} --mode html-text",
                    old.label("<old>"),
                    new.label("<new>")
                )),
            ))
        }
    }
}

struct Side<'a> {
    raw: &'a [u8],
    doc: &'a CanonicalDoc,
    tokens: &'a TokenDoc,
    key: [u8; 32],
}

impl Side<'_> {
    /// Raw offset where the piece belonging to token `index` starts: the end
    /// of the previous token, so the piece carries the markup and whitespace
    /// in front of the token.
    fn piece_start(&self, index: usize) -> usize {
        match index.checked_sub(1) {
            None => 0,
            Some(previous) => {
                let previous_end = self.tokens.tokens[previous].raw.end;
                match self.tokens.tokens.get(index) {
                    Some(token) => previous_end.min(token.raw.start),
                    None => previous_end,
                }
            }
        }
    }

    /// Raw span covered by a token range. An empty range is a position: the
    /// end of the token before it.
    fn span(&self, range: Range<usize>) -> Range<usize> {
        if range.is_empty() {
            let at = range
                .start
                .checked_sub(1)
                .map_or(0, |previous| self.tokens.tokens[previous].raw.end);
            return at..at;
        }
        let start = self.tokens.tokens[range.start].raw.start;
        let end = self.tokens.tokens[range.end - 1].raw.end;
        start..end.max(start)
    }

    fn digest(&self, span: Range<usize>) -> String {
        format!(
            "blake3-keyed:{}",
            blake3::keyed_hash(&self.key, &self.raw[span]).to_hex()
        )
    }

    /// Canonical text of a token range.
    fn text(&self, range: Range<usize>) -> &str {
        if range.is_empty() {
            return "";
        }
        let start = self.tokens.tokens[range.start].canon.start;
        let end = self.tokens.tokens[range.end - 1].canon.end;
        &self.doc.text[start..end]
    }
}

struct Sides<'a> {
    old: Side<'a>,
    new: Side<'a>,
    explicit: bool,
}

/// Turn token-level changes into report hunks, in document order, and add a
/// `formatting_only` hunk for each unchanged stretch whose raw bytes differ.
fn build_hunks(sides: &Sides<'_>, changes: &[Change]) -> Vec<Hunk> {
    let old_len = sides.old.tokens.tokens.len();
    let new_len = sides.new.tokens.tokens.len();
    let mut hunks = Vec::new();
    let mut old_cursor = 0;
    let mut new_cursor = 0;

    for change in changes {
        if let Some(hunk) = formatting_hunk(
            sides,
            old_cursor..change.old.start,
            new_cursor..change.new.start,
            false,
        ) {
            hunks.push(hunk);
        }
        hunks.push(semantic_hunk(sides, change));
        old_cursor = change.old.end;
        new_cursor = change.new.end;
    }
    if let Some(hunk) = formatting_hunk(sides, old_cursor..old_len, new_cursor..new_len, true) {
        hunks.push(hunk);
    }

    let width = hunks.len().to_string().len().max(3);
    for (index, hunk) in hunks.iter_mut().enumerate() {
        hunk.id = format!("hunk-{:0width$}", index + 1);
    }
    hunks
}

fn semantic_hunk(sides: &Sides<'_>, change: &Change) -> Hunk {
    let (class, numeric) = classify(
        sides.old.tokens,
        change.old.clone(),
        sides.new.tokens,
        change.new.clone(),
    );
    let old_span = sides.old.span(change.old.clone());
    let new_span = sides.new.span(change.new.clone());

    let (old_text, new_text, text_truncated) = if sides.explicit {
        let (old_text, old_cut) = excerpt(sides.old.text(change.old.clone()));
        let (new_text, new_cut) = excerpt(sides.new.text(change.new.clone()));
        (Some(old_text), Some(new_text), old_cut || new_cut)
    } else {
        (None, None, false)
    };

    Hunk {
        id: String::new(),
        class,
        numeric,
        old_span: [old_span.start, old_span.end],
        new_span: [new_span.start, new_span.end],
        old_digest: sides.old.digest(old_span),
        new_digest: sides.new.digest(new_span),
        old_tokens: sides.old.tokens.count_words(change.old.clone()),
        new_tokens: sides.new.tokens.count_words(change.new.clone()),
        old_text,
        new_text,
        text_truncated,
    }
}

/// Cut `text` to the excerpt limit on a character boundary.
fn excerpt(text: &str) -> (String, bool) {
    if text.len() <= EXCERPT_MAX_BYTES {
        return (text.to_owned(), false);
    }
    let mut end = EXCERPT_MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// Compare the raw bytes behind a stretch of tokens the diff found equal.
///
/// The two ranges have the same length and equal canonical text token for
/// token. Each token's piece is the raw bytes from the end of the previous
/// token to its own end. The hunk spans from the first piece that differs to
/// the last. `to_end` adds whatever follows the final token of each input.
fn formatting_hunk(
    sides: &Sides<'_>,
    old_range: Range<usize>,
    new_range: Range<usize>,
    to_end: bool,
) -> Option<Hunk> {
    debug_assert_eq!(old_range.len(), new_range.len());
    let (old, new) = (&sides.old, &sides.new);
    let piece = |side: &Side<'_>, index: usize| -> Range<usize> {
        let start = side.piece_start(index);
        start..side.tokens.tokens[index].raw.end.max(start)
    };
    let differs = |offset: usize| {
        old.raw[piece(old, old_range.start + offset)]
            != new.raw[piece(new, new_range.start + offset)]
    };
    let count = old_range.len();
    let first = (0..count).find(|&offset| differs(offset));
    let last = (0..count).rev().find(|&offset| differs(offset));

    let old_tail = old.piece_start(old_range.end)..old.raw.len();
    let new_tail = new.piece_start(new_range.end)..new.raw.len();
    let tail_differs = to_end && old.raw[old_tail.clone()] != new.raw[new_tail.clone()];

    let (old_span, new_span, words) = match (first, last, tail_differs) {
        (None, _, false) | (_, None, false) => return None,
        (None, _, true) | (_, None, true) => (old_tail, new_tail, 0..0),
        (Some(first), Some(last), tail) => {
            let old_start = piece(old, old_range.start + first).start;
            let new_start = piece(new, new_range.start + first).start;
            let (old_end, new_end) = if tail {
                (old.raw.len(), new.raw.len())
            } else {
                (
                    piece(old, old_range.start + last).end,
                    piece(new, new_range.start + last).end,
                )
            };
            (
                old_start..old_end.max(old_start),
                new_start..new_end.max(new_start),
                first..last + 1,
            )
        }
    };

    let word_count = old
        .tokens
        .count_words(old_range.start + words.start..old_range.start + words.end);
    Some(Hunk {
        id: String::new(),
        class: HunkClass::FormattingOnly,
        numeric: false,
        old_span: [old_span.start, old_span.end],
        new_span: [new_span.start, new_span.end],
        old_digest: old.digest(old_span),
        new_digest: new.digest(new_span),
        old_tokens: word_count,
        new_tokens: word_count,
        old_text: None,
        new_text: None,
        text_truncated: false,
    })
}

fn summarize(hunks: &[Hunk], old: &TokenDoc, new: &TokenDoc) -> Summary {
    let count = |class: HunkClass| hunks.iter().filter(|hunk| hunk.class == class).count();
    let formatting_only_hunks = count(HunkClass::FormattingOnly);
    let old_tokens = old.count_words(0..old.tokens.len());
    let new_tokens = new.count_words(0..new.tokens.len());
    let changed: usize = hunks
        .iter()
        .filter(|hunk| hunk.class.is_semantic())
        .map(|hunk| hunk.old_tokens + hunk.new_tokens)
        .sum();
    let total = old_tokens + new_tokens;
    let churn_ratio = if total == 0 {
        0.0
    } else {
        // Six decimal places: enough to rank documents, stable to print.
        (changed as f64 / total as f64 * 1_000_000.0).round() / 1_000_000.0
    };
    Summary {
        total_hunks: hunks.len(),
        semantic_hunks: hunks.len() - formatting_only_hunks,
        formatting_only_hunks,
        numeric_changed_hunks: count(HunkClass::NumericChanged),
        clause_added_hunks: count(HunkClass::ClauseAdded),
        clause_removed_hunks: count(HunkClass::ClauseRemoved),
        text_modified_hunks: count(HunkClass::TextModified),
        old_tokens,
        new_tokens,
        churn_ratio,
    }
}
