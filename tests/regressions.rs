//! Defects found by independent review before the first release, each kept
//! as a test. The first group is the serious kind: a visible text change that
//! was reported as `NO_SEMANTIC_CHANGE`.

use std::time::{Duration, Instant};

use redline::{HunkClass, Input, Mode, ModeChoice, Options, Outcome, Report, compare};

fn run_with(old: &str, new: &str, mode: ModeChoice) -> Report {
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
            mode,
            explicit: true,
        },
    )
    .expect("these inputs are comparable")
}

fn html(old: &str, new: &str) -> Report {
    run_with(old, new, ModeChoice::Fixed(Mode::HtmlText))
}

/// Class, numeric flag, old text and new text of every semantic hunk.
fn semantic(report: &Report) -> Vec<(HunkClass, bool, String, String)> {
    report
        .hunks
        .iter()
        .filter(|hunk| hunk.class.is_semantic())
        .map(|hunk| {
            (
                hunk.class,
                hunk.numeric,
                hunk.old_text.clone().unwrap_or_default(),
                hunk.new_text.clone().unwrap_or_default(),
            )
        })
        .collect()
}

fn entry(
    class: HunkClass,
    numeric: bool,
    old: &str,
    new: &str,
) -> (HunkClass, bool, String, String) {
    (class, numeric, old.to_owned(), new.to_owned())
}

// ── Missed changes ──────────────────────────────────────────────────────────

/// Each pair differs in text a browser renders.
const VISIBLE_CHANGES: &[(&str, &str)] = &[
    // Text after `<head>` implicitly opens the body.
    (
        "<html><head>Visible OLD text",
        "<html><head>Visible NEW text",
    ),
    (
        "<head><meta charset=utf-8>Price 100 dollars</html>",
        "<head><meta charset=utf-8>Price 999 dollars</html>",
    ),
    ("<head></p>alpha</div>more", "<head></p>beta</div>more"),
    (
        "<head><noscript><p>alpha</p>",
        "<head><noscript><p>beta</p>",
    ),
    ("<head/>alpha", "<head/>beta"),
    ("<head lang=en>alpha", "<head lang=en>beta"),
    // A `<head>` tag after body content is ignored.
    (
        "<body><p>intro</p><head>Rate is 5%<p>x</p></body>",
        "<body><p>intro</p><head>Rate is 9%<p>x</p></body>",
    ),
    ("<html>pre<head>OLD<body>z", "<html>pre<head>NEW<body>z"),
    // `--!>` closes a comment.
    (
        "<p>a</p><!-- x --!>VISIBLE OLD<!-- y --><p>b</p>",
        "<p>a</p><!-- x --!>VISIBLE NEW<!-- y --><p>b</p>",
    ),
    ("<p>a<!----!>one<!-- -->", "<p>a<!----!>two<!-- -->"),
    // Markup inside these elements is shown as literal text.
    (
        "<body><textarea><b>x</b></textarea>",
        "<body><textarea><i>x</i></textarea>",
    ),
    (
        "<body><textarea><!-- old --></textarea>",
        "<body><textarea><!-- new --></textarea>",
    ),
    ("<body><xmp><b>x</b></xmp>", "<body><xmp><i>x</i></xmp>"),
    ("<body><plaintext><b>x</b>", "<body><plaintext><i>x</i>"),
    // An unclosed `ix:header` is an ordinary element.
    (
        "<body><ix:header>a</ix:hidden>OLD text",
        "<body><ix:header>a</ix:hidden>NEW text",
    ),
    (
        "<body><ix:header a=b/>OLD text",
        "<body><ix:header a=b/>NEW text",
    ),
    // XHTML renders CDATA content.
    ("<p><![CDATA[old]]></p>", "<p><![CDATA[new]]></p>"),
    // Second review. In XHTML a self-closed title, script or style is a
    // complete element; read the HTML way it hid the rest of the document.
    (
        "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title/></head><body><p>Pay 100 now</p></body></html>",
        "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title/></head><body><p>Pay 900 now</p></body></html>",
    ),
    (
        "<html><head><script src=\"a.js\"/></head><body><p>Pay 100 now</p></body></html>",
        "<html><head><script src=\"a.js\"/></head><body><p>Pay 900 now</p></body></html>",
    ),
    (
        "<html><head><style/></head><body><p>Pay 100 now</p></body></html>",
        "<html><head><style/></head><body><p>Pay 900 now</p></body></html>",
    ),
    // A title whose end tag is misspelled must not swallow the body.
    (
        "<head><title>Doc</titel></head><body><p>Pay 100</p>",
        "<head><title>Doc</titel></head><body><p>Pay 900</p>",
    ),
    // `</` with no `>` anywhere after it.
    (
        "a </ b and then lots of visible OLD text\nmore text",
        "a </ b and then lots of visible NEW text\nmore text",
    ),
    ("Ratio is </3 OLD", "Ratio is </3 NEW"),
    // Third review. `<!` and `<?` with no `>` anywhere after them.
    (
        "<!DOCTYPE html><p>Total <! see note OLD amount 100",
        "<!DOCTYPE html><p>Total <! see note NEW amount 900",
    ),
    (
        "<!DOCTYPE html><p>Total <? see note OLD amount 100",
        "<!DOCTYPE html><p>Total <? see note NEW amount 900",
    ),
    // An unclosed head title must not reach a later inline SVG title.
    (
        "<!DOCTYPE html><html><head><title>Doc</head><body><p>Pay 100 now</p><svg><title>i</title></svg>",
        "<!DOCTYPE html><html><head><title>Doc</head><body><p>Pay 900 now</p><svg><title>i</title></svg>",
    ),
    (
        "<head><title>Doc<body><p>Pay 100</p><svg><title>i</title></svg>",
        "<head><title>Doc<body><p>Pay 900</p><svg><title>i</title></svg>",
    ),
];

/// Repeating a tag whose end never comes once made the scanner search to the
/// end of the input once per tag: 57 to 78 seconds for under 1 MB.
#[test]
fn repeated_unclosed_tags_are_not_quadratic() {
    const COUNT: usize = 60_000;
    for (opener, tail) in [
        ("<title>x ", ""),
        ("<ix:header>x ", ""),
        ("<title>x</titl> ", ""),
        ("<head><title>x ", "<body>"),
        ("<title>x ", "</title>"),
    ] {
        let page = |last: &str| format!("<html><body>{}{last}{tail}", opener.repeat(COUNT));
        let started = Instant::now();
        let report = html(&page(" OLD "), &page(" NEW "));
        let elapsed = started.elapsed();
        assert_eq!(report.outcome, Outcome::ChangesFound, "{opener}{tail}");
        assert!(
            elapsed < Duration::from_secs(20),
            "{opener}: took {elapsed:?}"
        );
    }
}

#[test]
fn visible_text_changes_in_broken_html_are_found() {
    for (old, new) in VISIBLE_CHANGES {
        let report = html(old, new);
        assert_eq!(
            report.outcome,
            Outcome::ChangesFound,
            "missed change: {old:?} -> {new:?}"
        );
        let mirrored = html(new, old);
        assert_eq!(mirrored.outcome, Outcome::ChangesFound);
    }
}

/// The fixes above must not turn hidden differences into changes.
#[test]
fn differences_nobody_sees_stay_formatting_only() {
    let pairs = [
        (
            "<p class=a>Hello&nbsp;there</p>",
            "<div><p class=b>Hello\u{a0}there</p></div>",
        ),
        (
            "<p>a</p><script>var x = 1;</script>",
            "<p>a</p><script>var y = 2;</script>",
        ),
        (
            "<p>a</p><script>never closed 1",
            "<p>a</p><script>never closed 2",
        ),
        (
            "<p>a</p><style>p{x:1}</style>b",
            "<p>a</p><style>p{x:2}</style>b",
        ),
        (
            "<body><ix:header><ix:hidden>1</ix:hidden></ix:header>a",
            "<body><ix:header><ix:hidden>2</ix:hidden></ix:header>a",
        ),
        (
            "<head><title>A</title></head><p>x",
            "<head><title>B</title></head><p>x",
        ),
        ("<p>a<!-- one -->b</p>", "<p>a<!-- two -->b</p>"),
        ("<p>a</ one>b</p>", "<p>a</ two>b</p>"),
    ];
    for (old, new) in pairs {
        assert_eq!(
            html(old, new).outcome,
            Outcome::NoSemanticChange,
            "{old:?} -> {new:?}"
        );
    }
}

/// `auto` once chose `html-text` for any input that mentioned `<head>` or
/// `<html>`, which hid everything in angle brackets and after the mention.
#[test]
fn auto_mode_reads_plain_text_that_mentions_tags_as_text() {
    let pairs = [
        (
            "Notes: the <head> block.\nPayment is $100 due now.\n",
            "Notes: the <head> block.\nPayment is $900 due now.\n",
        ),
        (
            "Notes on <html> docs.\nUse Vec<String> here.\n",
            "Notes on <html> docs.\nUse Vec<Integer> here.\n",
        ),
    ];
    for (old, new) in pairs {
        let report = run_with(old, new, ModeChoice::Auto);
        assert_eq!(report.mode, Some(Mode::Whitespace));
        assert_eq!(report.outcome, Outcome::ChangesFound, "{old:?}");
    }
}

/// `auto` once read anything that opened with a common tag as HTML. Markdown
/// with an HTML heading, plain text that opens with `<b>`, and old tagged
/// EDGAR text all lost what sat in angle brackets. An input that opens with a
/// tag but not as an HTML document is now refused, and the caller chooses.
#[test]
fn auto_mode_refuses_a_leading_tag_that_is_not_an_html_document() {
    let pairs = [
        (
            "<h1 align=\"center\">Tool</h1>\n\nUse `map<K, V>` here.\n",
            "<h1 align=\"center\">Tool</h1>\n\nUse `map<K, W>` here.\n",
        ),
        (
            "<b>Note</b> see vector<int> x",
            "<b>Note</b> see vector<long> x",
        ),
        (
            "<i> is less than <j> in the loop",
            "<i> is less than <k> in the loop",
        ),
        (
            "<DOCUMENT>\n<TYPE>EX-10\n<SEQUENCE>2\n<TEXT>\n<TABLE>\n<CAPTION>\n<S> <C>\nRevenue 100\n</TABLE>\nSee note <F1> and Section <A> of the plan.\n</TEXT>\n</DOCUMENT>\n",
            "<DOCUMENT>\n<TYPE>EX-10\n<SEQUENCE>2\n<TEXT>\n<TABLE>\n<CAPTION>\n<S> <C>\nRevenue 100\n</TABLE>\nSee note <F2> and Section <B> of the plan.\n</TEXT>\n</DOCUMENT>\n",
        ),
        (
            "<div class=\"a\"><p>Hello</p></div>",
            "<div class=\"b\"><p>Hello</p></div>",
        ),
    ];
    for (old, new) in pairs {
        let refusal = compare(
            Input {
                bytes: old.as_bytes(),
                path: Some("old file"),
            },
            Input {
                bytes: new.as_bytes(),
                path: Some("-new"),
            },
            &Options {
                mode: ModeChoice::Auto,
                explicit: false,
            },
        )
        .expect_err("auto must not guess");
        assert_eq!(refusal.code, "E_MODE_AMBIGUOUS", "{old:?}");
        assert_eq!(refusal.detail["old"], "unclear");
        // The suggested command quotes the space and guards the leading dash.
        assert_eq!(
            refusal.next_command.as_deref(),
            Some("redline 'old file' ./-new --mode html-text")
        );
        // Read as text, as the caller would for Markdown or tagged text, the
        // change is found.
        let as_text = run_with(old, new, ModeChoice::Fixed(Mode::Whitespace));
        assert_eq!(as_text.outcome, Outcome::ChangesFound, "{old:?}");
    }
}

/// A document saved from EDGAR keeps its wrapper lines in front of the HTML.
#[test]
fn auto_mode_reads_an_edgar_wrapped_document_as_html() {
    let wrap = |body: &str| {
        format!(
            "<DOCUMENT>\n<TYPE>10-K\n<SEQUENCE>1\n<FILENAME>a.htm\n<DESCRIPTION>10-K\n<TEXT>\n\
             <!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\">\n<html><body>{body}</body></html>"
        )
    };
    let old = wrap("<p style=\"a\">Net income rose.</p>");
    let restyled = wrap("<p style=\"b\">Net income rose.</p>");
    let edited = wrap("<p style=\"a\">Net income fell.</p>");
    let same = run_with(&old, &restyled, ModeChoice::Auto);
    assert_eq!(same.mode, Some(Mode::HtmlText));
    assert_eq!(same.outcome, Outcome::NoSemanticChange);
    assert_eq!(
        semantic(&run_with(&old, &edited, ModeChoice::Auto)),
        [entry(HunkClass::TextModified, false, "rose", "fell")]
    );
}

// ── Span placement and classification ───────────────────────────────────────

/// In `raw` mode an added or removed line is the line and its newline, not
/// the previous line's newline plus the text.
#[test]
fn raw_mode_reports_whole_lines() {
    let raw = ModeChoice::Fixed(Mode::Raw);
    let added = run_with("a\nb\n", "a\nx\nb\n", raw);
    assert_eq!(
        semantic(&added),
        [entry(HunkClass::ClauseAdded, false, "", "x\n")]
    );
    assert_eq!(added.hunks[0].new_span, [2, 4]);
    let removed = run_with("a\nb\nc\n", "a\nc\n", raw);
    assert_eq!(
        semantic(&removed),
        [entry(HunkClass::ClauseRemoved, false, "b\n", "")]
    );
    assert_eq!(removed.hunks[0].old_span, [2, 4]);
}

/// Edits to the sign, month or magnitude of a number change the number.
#[test]
fn edits_to_what_a_number_means_are_numeric() {
    use HunkClass::{NumericChanged, TextModified};
    let ws = ModeChoice::Fixed(Mode::Whitespace);
    let cases = [
        (
            "value 5 here",
            "value -5 here",
            entry(NumericChanged, true, "", "-"),
        ),
        (
            "due March 1, 2024 ok",
            "due April 1, 2024 ok",
            entry(NumericChanged, true, "March", "April"),
        ),
        (
            "pay $5 million now",
            "pay $5 billion now",
            entry(NumericChanged, true, "million", "billion"),
        ),
        (
            "up 5 percent",
            "up 5 bps",
            entry(NumericChanged, true, "percent", "bps"),
        ),
        // The same words away from a number are ordinary text.
        (
            "the march flower",
            "the june flower",
            entry(TextModified, false, "march", "june"),
        ),
        // A list marker after a number is not its sign.
        (
            "Rate is 5\n- item\n",
            "Rate is 5\nitem\n",
            entry(TextModified, false, "- ", ""),
        ),
        // `may` is a verb far more often than a month.
        (
            "Bank 5 be late",
            "Bank 5 may be late",
            entry(TextModified, false, "", "may "),
        ),
        // A spacing edit next to a number does not change the number.
        (
            "up 5.25% today",
            "up 5.25 % today",
            entry(TextModified, false, "", " "),
        ),
        // A word that is not a modifier, next to a number, is ordinary text.
        (
            "pay 5 dollars",
            "pay 5 euros",
            entry(TextModified, false, "dollars", "euros"),
        ),
    ];
    for (old, new, expected) in cases {
        assert_eq!(semantic(&run_with(old, new, ws)), [expected], "{old:?}");
    }
    let parens: Vec<bool> = run_with("net 5 total", "net (5) total", ws)
        .hunks
        .iter()
        .map(|hunk| hunk.numeric)
        .collect();
    assert!(!parens.is_empty() && parens.iter().all(|&numeric| numeric));
}

/// Each item of a single-spaced list is its own block, so an added item is
/// reported as that item.
#[test]
fn an_added_list_item_is_a_clause() {
    let ws = ModeChoice::Fixed(Mode::Whitespace);
    // (old, new, added text, whether the added text holds a number)
    for (old, new, added, numeric) in [
        (
            "Terms:\n- Net 30\n- FOB origin\n",
            "Terms:\n- Net 30\n- No refunds\n- FOB origin\n",
            " - No refunds",
            false,
        ),
        (
            "Steps:\n1. Sign here\n2. Send it\n",
            "Steps:\n1. Sign here\n2. Date it\n2. Send it\n",
            " 2. Date it",
            true,
        ),
        (
            "Parties:\n(a) the Buyer\n(b) the Seller\n",
            "Parties:\n(a) the Buyer\n(b) the Agent\n(b) the Seller\n",
            " (b) the Agent",
            false,
        ),
    ] {
        assert_eq!(
            semantic(&run_with(old, new, ws)),
            [entry(HunkClass::ClauseAdded, numeric, "", added)],
            "{old:?}"
        );
    }
}

// ── Cost ────────────────────────────────────────────────────────────────────

/// Join `count` distinct words starting at `first`, `per_block` to a
/// paragraph (0 for one paragraph).
fn words(prefix: &str, count: usize, per_block: usize) -> String {
    let mut out = String::new();
    for index in 0..count {
        if index > 0 {
            let block_break = per_block > 0 && index % per_block == 0;
            out.push_str(if block_break { "\n\n" } else { " " });
        }
        out.push_str(prefix);
        out.push_str(&index.to_string());
    }
    out
}

/// Heavily changed input once took time quadratic in its size: minutes for a
/// few megabytes. These pairs took 16 to 46 seconds in a release build before
/// the fix and well under a second after it. The limit is loose enough for an
/// unoptimized build on a busy machine and still far below the old cost.
#[test]
fn heavily_changed_input_is_not_quadratic() {
    const COUNT: usize = 60_000;
    let ws = ModeChoice::Fixed(Mode::Whitespace);
    let every_other: String = (0..COUNT)
        .map(|index| {
            let prefix = if index % 2 == 0 { "v" } else { "w" };
            format!("{prefix}{index}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let cases = [
        ("all different", words("w", COUNT, 0), words("v", COUNT, 0)),
        ("paragraphs", words("w", COUNT, 100), words("v", COUNT, 100)),
        ("every other word", words("w", COUNT, 0), every_other),
    ];
    for (name, old, new) in cases {
        let started = Instant::now();
        let report = run_with(&old, &new, ws);
        let elapsed = started.elapsed();
        assert_eq!(report.outcome, Outcome::ChangesFound);
        assert!(
            elapsed < Duration::from_secs(20),
            "{name}: took {elapsed:?}"
        );
        // Nothing is lost when a region is reported coarsely: the hunks
        // still cover every changed word.
        let summary = report.summary.expect("a verdict has a summary");
        assert!(
            summary.churn_ratio > 0.45,
            "{name}: {}",
            summary.churn_ratio
        );
    }
}

/// A long run of a few repeated values (a column of `yes` and `no` cells)
/// once took time quadratic in its length even with two edits: 27 seconds
/// for 560 KB in a release build. The edits must also still be found one by
/// one, not folded into a replacement of everything between them.
#[test]
fn repeated_values_with_a_few_edits_are_fast_and_exact() {
    const COUNT: usize = 60_000;
    // A fixed, irregular yes/no sequence.
    let mut state = 5u64;
    let lines: Vec<&str> = (0..COUNT)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if (state >> 40) & 1 == 0 { "yes" } else { "no" }
        })
        .collect();
    let mut edited = lines.clone();
    edited[100] = "maybe";
    edited[COUNT - 100] = "perhaps";

    for (mode, separator) in [(Mode::Raw, "\n"), (Mode::Whitespace, "\n\n")] {
        let old = lines.join(separator);
        let new = edited.join(separator);
        let started = Instant::now();
        let report = run_with(&old, &new, ModeChoice::Fixed(mode));
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(20),
            "{mode:?}: took {elapsed:?}"
        );
        let found: Vec<String> = semantic(&report)
            .into_iter()
            .map(|(_, _, _, new_text)| new_text)
            .collect();
        assert_eq!(found, ["maybe", "perhaps"], "{mode:?}");
    }
}

/// A large region is split on unique words before the word diff runs. The
/// split must not change what is found: a few scattered edits in a long
/// single paragraph are still reported one by one.
#[test]
fn scattered_edits_in_a_long_paragraph_are_reported_individually() {
    // The same three edits in a paragraph of `count` words, at the same
    // relative places. Digits are dropped from the result so the two sizes
    // can be compared.
    let shapes = |count: usize| {
        let (first, second, third) = (count / 4, count * 3 / 5, count * 19 / 20);
        let old = words("w", count, 0);
        let new = old
            .replace(&format!("w{first} "), &format!("CHANGED{first} "))
            .replace(&format!("w{second} "), &format!("w{second} ADDED "))
            .replace(&format!(" w{third} "), " ");
        let strip = |text: String| text.replace(|ch: char| ch.is_ascii_digit(), "");
        semantic(&run_with(&old, &new, ModeChoice::Fixed(Mode::Whitespace)))
            .into_iter()
            .map(|(class, numeric, old, new)| (class, numeric, strip(old), strip(new)))
            .collect::<Vec<_>>()
    };
    // 200 words go straight to the word diff; 20,000 take the anchored path.
    let direct = shapes(200);
    assert_eq!(direct.len(), 3, "{direct:?}");
    assert_eq!(
        direct[0],
        entry(HunkClass::NumericChanged, true, "w", "CHANGED")
    );
    assert_eq!(shapes(20_000), direct);
}
