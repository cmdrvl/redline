//! The invariants `docs/SPEC.md` section 6 requires before a release.
//!
//! Each property runs over generated documents, including deliberately
//! broken markup, in every mode.

use proptest::prelude::*;
use redline::diff::diff;
use redline::tokens::{Interner, tokenize};
use redline::{
    Edges, Hunk, HunkClass, Input, Mode, ModeChoice, Options, Outcome, Report, canonicalize,
    compare, hunk_digest,
};

const MODES: [Mode; 3] = [Mode::Raw, Mode::Whitespace, Mode::HtmlText];

fn run(old: &str, new: &str, mode: Mode, explicit: bool) -> Report {
    compare(
        Input {
            bytes: old.as_bytes(),
            path: None,
        },
        Input {
            bytes: new.as_bytes(),
            path: None,
        },
        &Options {
            mode: ModeChoice::Fixed(mode),
            explicit,
        },
    )
    .expect("non-empty UTF-8 inputs never refuse under a fixed mode")
}

// ── Generators ──────────────────────────────────────────────────────────────

/// Words that cannot collide with any key or constant in the report JSON.
const WORDS: &[&str] = &[
    "Qzalpha", "Qzbravo", "Qzcedar", "Qzdelta", "Qzember", "Qzfjord", "Qzgrove", "Qzhaven",
    "Qzindex", "Qzjoule", "Qzkappa", "Qzlumen", "Qzmango", "Qznorth", "Qzolive", "Qzpiano",
];

const NUMBERS: &[&str] = &["5.25", "1,250,000.00", "2024", "17", "0.5", "42"];

fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => prop::sample::select(WORDS).prop_map(str::to_owned),
        1 => prop::sample::select(NUMBERS).prop_map(str::to_owned),
    ]
}

fn sentence() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(word(), 1..7)
}

/// A document as paragraphs of sentences of words.
fn outline() -> impl Strategy<Value = Vec<Vec<Vec<String>>>> {
    prop::collection::vec(prop::collection::vec(sentence(), 1..4), 1..5)
}

fn render_text(outline: &[Vec<Vec<String>>]) -> String {
    outline
        .iter()
        .map(|paragraph| {
            paragraph
                .iter()
                .map(|sentence| format!("{}.", sentence.join(" ")))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render the same text as HTML. `style` varies markup only, never text.
fn render_html(outline: &[Vec<Vec<String>>], style: u8) -> String {
    let mut out = String::new();
    if style & 1 == 1 {
        out.push_str("<html><head><title>Qzhidden</title><style>p{margin:0}</style></head><body>");
    }
    for (index, paragraph) in outline.iter().enumerate() {
        let tag = if (style >> 1) & 1 == 1 { "div" } else { "p" };
        if (style >> 2) & 1 == 1 {
            out.push_str(&format!(
                "<{tag} style=\"margin:{index}px\" class='c{style}'>"
            ));
        } else {
            out.push_str(&format!("<{tag}>"));
        }
        for (position, sentence) in paragraph.iter().enumerate() {
            if position > 0 {
                out.push_str(if (style >> 3) & 1 == 1 {
                    "\n  "
                } else {
                    "&#32;"
                });
            }
            for (word_index, word) in sentence.iter().enumerate() {
                if word_index > 0 {
                    out.push_str(if (style >> 4) & 1 == 1 { " " } else { "&nbsp;" });
                }
                if (style >> 5) & 1 == 1 && word_index % 2 == 0 {
                    out.push_str(&format!("<span data-i=\"{word_index}\">{word}</span>"));
                } else {
                    out.push_str(word);
                }
            }
            out.push_str(if (style >> 6) & 1 == 1 { "&#46;" } else { "." });
        }
        out.push_str(&format!("</{tag}>"));
        if (style >> 7) & 1 == 1 {
            out.push_str("<!-- Qzcomment -->\n<script>var Qzscript = '<p>';</script>");
        }
    }
    out
}

/// Soup of markup fragments, to stress the scanner on broken input.
fn soup() -> impl Strategy<Value = String> {
    const PIECES: &[&str] = &[
        "<",
        ">",
        "&",
        ";",
        "\"",
        "'",
        "=",
        "/",
        " ",
        "\n",
        "\r\n",
        "\n\n",
        "\t",
        "a",
        "b",
        "Qz",
        "é",
        "\u{feff}",
        "\u{2029}",
        "1",
        "2.5",
        ".",
        ",",
        "<p>",
        "</p>",
        "<br/>",
        "<b>",
        "</b>",
        "<div class=\"x>y\">",
        "</div>",
        "<!--",
        "-->",
        "<!-- c -->",
        "<script>",
        "</script>",
        "<style>",
        "</style>",
        "<head>",
        "</head>",
        "<body>",
        "<title>",
        "</title>",
        "<noscript>",
        "<ix:header>",
        "</ix:header>",
        "&amp;",
        "&amp",
        "&nbsp;",
        "&#65;",
        "&#x41",
        "&#",
        "&notit;",
        "&bogus;",
        "<a href='q",
        "<?xml",
        "?>",
        "<![CDATA[",
        "]]>",
        "<td>",
        "</td>",
        "</",
    ];
    prop::collection::vec(prop::sample::select(PIECES), 1..40).prop_map(|pieces| pieces.concat())
}

fn document_pair() -> impl Strategy<Value = (String, String)> {
    prop_oneof![
        (outline(), outline()).prop_map(|(a, b)| (render_text(&a), render_text(&b))),
        (outline(), outline(), any::<u8>(), any::<u8>())
            .prop_map(|(a, b, sa, sb)| (render_html(&a, sa), render_html(&b, sb))),
        (outline(), any::<u8>(), any::<u8>())
            .prop_map(|(a, sa, sb)| (render_html(&a, sa), render_html(&a, sb))),
        (soup(), soup()),
        soup().prop_flat_map(|a| {
            let len = a.len();
            (Just(a), 0..=len, soup()).prop_map(|(a, at, insert)| {
                let mut at = at;
                while !a.is_char_boundary(at) {
                    at -= 1;
                }
                let mut b = a.clone();
                b.insert_str(at, &insert);
                (a, b)
            })
        }),
    ]
}

// ── Checks ──────────────────────────────────────────────────────────────────

/// Invariant 1 for one report, built with `explicit` so hunk text is present.
fn check_spans(old: &str, new: &str, mode: Mode, report: &Report) -> Result<(), TestCaseError> {
    let mut old_floor = 0;
    let mut new_floor = 0;
    for hunk in &report.hunks {
        let [old_start, old_end] = hunk.old_span;
        let [new_start, new_end] = hunk.new_span;
        prop_assert!(
            old_start <= old_end && old_end <= old.len(),
            "old span out of range"
        );
        prop_assert!(
            new_start <= new_end && new_end <= new.len(),
            "new span out of range"
        );
        prop_assert!(
            old.is_char_boundary(old_start) && old.is_char_boundary(old_end),
            "old span splits a character"
        );
        prop_assert!(
            new.is_char_boundary(new_start) && new.is_char_boundary(new_end),
            "new span splits a character"
        );
        prop_assert!(old_start >= old_floor, "old spans out of order: {hunk:?}");
        prop_assert!(new_start >= new_floor, "new spans out of order: {hunk:?}");
        old_floor = old_end;
        new_floor = new_end;

        let old_slice = &old[old_start..old_end];
        let new_slice = &new[new_start..new_end];
        prop_assert_eq!(
            &hunk.old_digest,
            &hunk_digest(old.as_bytes(), old_start..old_end)
        );
        prop_assert_eq!(
            &hunk.new_digest,
            &hunk_digest(new.as_bytes(), new_start..new_end)
        );

        if hunk.class == HunkClass::FormattingOnly {
            prop_assert_ne!(old_slice, new_slice, "formatting hunk with equal raw bytes");
        }
        // Re-reading a slice on its own only works where markup means the
        // same thing in and out of context. Inside literal-text content it
        // does not: `<b>` is text in a `<textarea>` and a tag outside one.
        // Those documents keep every check above; what their spans hold is
        // pinned by `spans_inside_literal_text_cover_the_literal_bytes`.
        if mode == Mode::HtmlText && (has_literal_text(old) || has_literal_text(new)) {
            continue;
        }
        if hunk.class == HunkClass::FormattingOnly {
            prop_assert_eq!(
                canonicalize(old_slice, mode, Edges::Document).text,
                canonicalize(new_slice, mode, Edges::Document).text,
                "formatting hunk whose canonical text differs"
            );
        } else {
            prop_assert!(
                !hunk.text_truncated,
                "generated hunks stay under the excerpt limit"
            );
            let old_text = hunk.old_text.as_deref().expect("explicit run");
            let new_text = hunk.new_text.as_deref().expect("explicit run");
            prop_assert_ne!(old_text, new_text, "semantic hunk with equal text");
            prop_assert_eq!(
                canonicalize(old_slice, mode, Edges::Fragment).text,
                old_text,
                "old span does not re-canonicalize to the hunk text"
            );
            prop_assert_eq!(
                canonicalize(new_slice, mode, Edges::Fragment).text,
                new_text,
                "new span does not re-canonicalize to the hunk text"
            );
        }
    }
    Ok(())
}

/// True when `source` opens an element or section whose content is literal
/// text, where markup is not markup.
fn has_literal_text(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    ["<![cdata[", "<textarea", "<xmp", "<plaintext", "<title"]
        .iter()
        .any(|opener| lower.contains(opener))
        // Markup that never closes keeps the rest of the input as text.
        || ["<!", "<?", "</"].iter().any(|opener| {
            lower
                .match_indices(opener)
                .any(|(at, _)| !lower[at..].contains('>'))
        })
}

/// Inside literal-text content a span still holds exactly the changed bytes.
#[test]
fn spans_inside_literal_text_cover_the_literal_bytes() {
    let cases = [
        (
            "<p>a <![CDATA[x <b> y]]> c</p>",
            "<p>a <![CDATA[x <i> y]]> c</p>",
            "b",
            "i",
        ),
        (
            "<body><textarea>keep <em>old</em></textarea>",
            "<body><textarea>keep <em>new</em></textarea>",
            "old",
            "new",
        ),
        (
            "<body><xmp>&amp; one</xmp>z",
            "<body><xmp>&amp; two</xmp>z",
            "one",
            "two",
        ),
    ];
    for (old, new, old_bytes, new_bytes) in cases {
        let report = run(old, new, Mode::HtmlText, true);
        assert_eq!(report.hunks.len(), 1, "{old}");
        let hunk = &report.hunks[0];
        assert_eq!(&old[hunk.old_span[0]..hunk.old_span[1]], old_bytes);
        assert_eq!(&new[hunk.new_span[0]..hunk.new_span[1]], new_bytes);
        assert_eq!(hunk.old_text.as_deref(), Some(old_bytes));
        assert_eq!(hunk.new_text.as_deref(), Some(new_bytes));
    }
}

/// What the report for `(new, old)` must be, given the report for `(old, new)`.
fn mirrored(report: &Report) -> Report {
    let mut mirror = report.clone();
    std::mem::swap(&mut mirror.inputs.old, &mut mirror.inputs.new);
    if let Some(summary) = &mut mirror.summary {
        std::mem::swap(&mut summary.old_tokens, &mut summary.new_tokens);
        std::mem::swap(
            &mut summary.clause_added_hunks,
            &mut summary.clause_removed_hunks,
        );
    }
    for hunk in &mut mirror.hunks {
        let Hunk {
            class,
            old_span,
            new_span,
            old_digest,
            new_digest,
            old_tokens,
            new_tokens,
            old_text,
            new_text,
            ..
        } = hunk;
        std::mem::swap(old_span, new_span);
        std::mem::swap(old_digest, new_digest);
        std::mem::swap(old_tokens, new_tokens);
        std::mem::swap(old_text, new_text);
        *class = match *class {
            HunkClass::ClauseAdded => HunkClass::ClauseRemoved,
            HunkClass::ClauseRemoved => HunkClass::ClauseAdded,
            other => other,
        };
    }
    mirror
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// Invariant 1: every span slices the original bytes on character
    /// boundaries and re-canonicalizes to the text that was diffed.
    #[test]
    fn spans_are_exact((old, new) in document_pair()) {
        for mode in MODES {
            let report = run(&old, &new, mode, true);
            check_spans(&old, &new, mode, &report)?;
        }
    }

    /// The changes the diff reports account for the whole difference:
    /// applying them to the old tokens yields the new tokens.
    #[test]
    fn changes_are_complete((old, new) in document_pair()) {
        for mode in MODES {
            let old_doc = canonicalize(&old, mode, Edges::Document);
            let new_doc = canonicalize(&new, mode, Edges::Document);
            let mut interner = Interner::default();
            let old_tokens = tokenize(&old_doc, mode, &mut interner);
            let new_tokens = tokenize(&new_doc, mode, &mut interner);
            let ids = |tokens: &[redline::tokens::Token]| -> Vec<u32> {
                tokens.iter().map(|token| token.id).collect()
            };
            let mut rebuilt = Vec::new();
            let mut cursor = 0;
            for change in diff(&old_tokens, &new_tokens) {
                prop_assert!(change.old.start >= cursor, "changes overlap");
                prop_assert!(!(change.old.is_empty() && change.new.is_empty()), "empty change");
                rebuilt.extend(ids(&old_tokens.tokens[cursor..change.old.start]));
                rebuilt.extend(ids(&new_tokens.tokens[change.new.clone()]));
                cursor = change.old.end;
            }
            rebuilt.extend(ids(&old_tokens.tokens[cursor..]));
            prop_assert_eq!(rebuilt, ids(&new_tokens.tokens));
        }
    }

    /// Invariant 2a: a document against itself has no hunks.
    #[test]
    fn identical_inputs_are_unchanged((old, _new) in document_pair()) {
        for mode in MODES {
            let report = run(&old, &old, mode, false);
            prop_assert_eq!(report.outcome, Outcome::NoSemanticChange);
            prop_assert!(report.hunks.is_empty());
        }
    }

    /// Invariant 2b: swapping the inputs mirrors the report exactly.
    #[test]
    fn swapping_inputs_mirrors_the_report((old, new) in document_pair()) {
        for mode in MODES {
            let forward = run(&old, &new, mode, true);
            let backward = run(&new, &old, mode, true);
            prop_assert_eq!(mirrored(&forward), backward);
        }
    }

    /// Same inputs, same bytes out.
    #[test]
    fn output_is_deterministic((old, new) in document_pair()) {
        for mode in MODES {
            let first = serde_json::to_string(&run(&old, &new, mode, true)).unwrap();
            let second = serde_json::to_string(&run(&old, &new, mode, true)).unwrap();
            prop_assert_eq!(first, second);
        }
    }

    /// Invariant 3 (HTML): restyling, re-tagging, re-spacing and re-encoding
    /// the same text is never a semantic change.
    #[test]
    fn html_markup_changes_are_formatting_only(
        outline in outline(),
        old_style in any::<u8>(),
        new_style in any::<u8>(),
    ) {
        let old = render_html(&outline, old_style);
        let new = render_html(&outline, new_style);
        let report = run(&old, &new, Mode::HtmlText, true);
        prop_assert_eq!(report.outcome, Outcome::NoSemanticChange, "{:?}", report.hunks);
        prop_assert!(report.hunks.iter().all(|hunk| hunk.class == HunkClass::FormattingOnly));
        prop_assert_eq!(report.hunks.is_empty(), old == new);
    }

    /// Invariant 3 (whitespace): re-wrapping is never a semantic change.
    #[test]
    fn rewrapping_is_formatting_only(outline in outline(), width in 1usize..6) {
        let old = render_text(&outline);
        // Break after every `width`th word, turn the rest into mixed runs.
        let mut new = String::from("  ");
        for paragraph in old.split("\n\n") {
            for (index, piece) in paragraph.split(' ').enumerate() {
                new.push_str(piece);
                new.push_str(if index % width == width - 1 { " \r\n\t" } else { "  " });
            }
            new.push_str("\n \n");
        }
        let report = run(&old, &new, Mode::Whitespace, true);
        prop_assert_eq!(report.outcome, Outcome::NoSemanticChange, "{:?}", report.hunks);
        prop_assert!(report.hunks.iter().all(|hunk| hunk.class == HunkClass::FormattingOnly));
        prop_assert!(!report.hunks.is_empty());
        // The same edit is a change when every byte counts.
        prop_assert_eq!(run(&old, &new, Mode::Raw, false).outcome, Outcome::ChangesFound);
    }

    /// Invariant 4: the default report carries no input text.
    #[test]
    fn default_report_retains_no_input_text(
        old_outline in outline(),
        new_outline in outline(),
        style in any::<u8>(),
    ) {
        let cases = [
            (render_text(&old_outline), render_text(&new_outline), Mode::Whitespace),
            (render_text(&old_outline), render_text(&new_outline), Mode::Raw),
            (render_html(&old_outline, style), render_html(&new_outline, !style), Mode::HtmlText),
        ];
        for (old, new, mode) in cases {
            let json = serde_json::to_string(&run(&old, &new, mode, false)).unwrap();
            prop_assert!(!json.contains("Qz"), "input word leaked: {json}");
            for number in ["5.25", "1,250,000.00"] {
                prop_assert!(!json.contains(number), "input number leaked: {json}");
            }
        }
    }
}

/// The zero-retention property is checked against a report that would leak if
/// redaction were off: the explicit report for the same inputs does carry text.
#[test]
fn explicit_report_carries_the_text_the_default_omits() {
    let old = "Qzalpha pays 5.25 percent.";
    let new = "Qzbravo pays 5.50 percent.";
    let explicit = serde_json::to_string(&run(old, new, Mode::Whitespace, true)).unwrap();
    assert!(explicit.contains("Qzalpha") && explicit.contains("5.50"));
    let default = serde_json::to_string(&run(old, new, Mode::Whitespace, false)).unwrap();
    assert!(!default.contains("Qz") && !default.contains("5.25") && !default.contains("5.50"));
}

/// Excerpts are cut at the limit, on a character boundary, and say so.
#[test]
fn long_excerpts_are_truncated_and_flagged() {
    let old = "é".repeat(3000);
    let new = "ü".repeat(3000);
    let report = run(&old, &new, Mode::Whitespace, true);
    let hunk = &report.hunks[0];
    assert!(hunk.text_truncated);
    assert!(hunk.old_text.as_deref().unwrap().len() <= redline::engine::EXCERPT_MAX_BYTES);
    assert_eq!(hunk.old_span, [0, old.len()]);
}

fn classes(old: &str, new: &str, mode: Mode) -> Vec<(HunkClass, bool)> {
    run(old, new, mode, false)
        .hunks
        .iter()
        .map(|hunk| (hunk.class, hunk.numeric))
        .collect()
}

#[test]
fn classification_examples() {
    use HunkClass::*;
    let ws = Mode::Whitespace;
    assert_eq!(
        classes("Rate is 5.25%.", "Rate is 5.50%.", ws),
        [(NumericChanged, true)]
    );
    assert_eq!(
        classes("The big dog.", "The small dog.", ws),
        [(TextModified, false)]
    );
    assert_eq!(
        classes(
            "One here. Three here.",
            "One here. Two here. Three here.",
            ws
        ),
        [(ClauseAdded, false)]
    );
    assert_eq!(
        classes(
            "One here. Pay 7 now. Three here.",
            "One here. Three here.",
            ws
        ),
        [(ClauseRemoved, true)]
    );
    assert_eq!(
        classes("<p>A b.</p>", "<div class=x>A  b.</div>", Mode::HtmlText),
        [(FormattingOnly, false)]
    );
    assert_eq!(
        classes(
            "<p>Due March 1, 2024.</p>",
            "<p>Due April 15, 2025.</p>",
            Mode::HtmlText
        ),
        [(NumericChanged, true)]
    );
}

#[test]
fn spans_point_at_the_change_in_the_original_html() {
    let old = "<html><body><p>The fee is <b>$1,250.00</b> per&nbsp;year.</p></body></html>";
    let new =
        "<html><body><p class=\"x\">The fee is <b>$1,500.00</b> per&nbsp;year.</p></body></html>";
    let report = run(old, new, Mode::HtmlText, true);
    let semantic: Vec<&Hunk> = report
        .hunks
        .iter()
        .filter(|h| h.class.is_semantic())
        .collect();
    assert_eq!(semantic.len(), 1);
    let hunk = semantic[0];
    assert_eq!(&old[hunk.old_span[0]..hunk.old_span[1]], "1,250.00");
    assert_eq!(&new[hunk.new_span[0]..hunk.new_span[1]], "1,500.00");
    assert_eq!(report.outcome, Outcome::ChangesFound);
    let summary = report.summary.unwrap();
    assert_eq!(
        (summary.semantic_hunks, summary.formatting_only_hunks),
        (1, 1)
    );
}

#[test]
fn refusals_are_typed() {
    let refuse = |old: &[u8], new: &[u8], mode: ModeChoice| {
        compare(
            Input {
                bytes: old,
                path: Some("old"),
            },
            Input {
                bytes: new,
                path: Some("new"),
            },
            &Options {
                mode,
                explicit: false,
            },
        )
        .expect_err("must refuse")
    };
    assert_eq!(refuse(b"", b"x", ModeChoice::Auto).code, "E_EMPTY_INPUT");
    assert_eq!(
        refuse(b"x", b"\xff\xfe", ModeChoice::Auto).code,
        "E_BINARY_INPUT"
    );
    let pdf = refuse(b"%PDF-1.7\n\x00\x01", b"x", ModeChoice::Auto);
    assert_eq!(pdf.code, "E_BINARY_INPUT");
    assert!(pdf.next_command.unwrap().starts_with("pdftotext"));
    let mixed = refuse(b"<html><p>x</p></html>", b"x", ModeChoice::Auto);
    assert_eq!(mixed.code, "E_MODE_AMBIGUOUS");
    assert_eq!(
        ModeChoice::parse("markdown").unwrap_err().code,
        "E_MODE_UNSUPPORTED"
    );
    assert_eq!(
        ModeChoice::parse("html-text"),
        Ok(ModeChoice::Fixed(Mode::HtmlText))
    );
}
