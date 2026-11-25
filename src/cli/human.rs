//! Human-readable output. The wording here is part of the public contract.

use std::fmt::Write;

use crate::refusal::Refusal;
use crate::report::{Outcome, Report};

pub fn render(report: &Report) -> String {
    match &report.refusal {
        Some(refusal) => render_refusal(report, refusal),
        None => render_verdict(report),
    }
}

fn compared(report: &Report) -> String {
    format!(
        "Compared: {} -> {}",
        report.inputs.old.path.as_deref().unwrap_or("<old>"),
        report.inputs.new.path.as_deref().unwrap_or("<new>")
    )
}

fn render_verdict(report: &Report) -> String {
    let mut out = String::new();
    let headline = match report.outcome {
        Outcome::NoSemanticChange => "NO SEMANTIC CHANGE",
        Outcome::ChangesFound => "CHANGES FOUND",
        Outcome::Refusal => "REFUSAL",
    };
    // Writing to a String cannot fail.
    let _ = writeln!(out, "REDLINE\n\n{headline}\n\n{}", compared(report));
    if let Some(mode) = report.mode {
        let _ = writeln!(out, "Mode: {}", mode.as_str());
    }
    if let Some(summary) = &report.summary {
        let _ = writeln!(
            out,
            "Hunks: {} semantic ({} clause_added, {} clause_removed, {} numeric_changed, {} text_modified), {} formatting_only",
            summary.semantic_hunks,
            summary.clause_added_hunks,
            summary.clause_removed_hunks,
            summary.numeric_changed_hunks,
            summary.text_modified_hunks,
            summary.formatting_only_hunks
        );
        let _ = writeln!(
            out,
            "Tokens: old={} new={} churn={:.2}%",
            summary.old_tokens,
            summary.new_tokens,
            summary.churn_ratio * 100.0
        );
    }
    if !report.hunks.is_empty() {
        out.push('\n');
    }
    for hunk in &report.hunks {
        let _ = writeln!(
            out,
            "{}  {:<15}  old[{},{})  new[{},{})  tokens {} -> {}{}",
            hunk.id,
            hunk.class.as_str(),
            hunk.old_span[0],
            hunk.old_span[1],
            hunk.new_span[0],
            hunk.new_span[1],
            hunk.old_tokens,
            hunk.new_tokens,
            if hunk.numeric { "  numeric" } else { "" }
        );
        if let Some(text) = hunk.old_text.as_deref().filter(|text| !text.is_empty()) {
            let _ = writeln!(out, "  - {text}");
        }
        if let Some(text) = hunk.new_text.as_deref().filter(|text| !text.is_empty()) {
            let _ = writeln!(out, "  + {text}");
        }
        if hunk.text_truncated {
            let _ = writeln!(
                out,
                "  (text cut at the excerpt limit; slice the file at the span for the rest)"
            );
        }
    }
    let has_semantic = report.hunks.iter().any(|hunk| hunk.class.is_semantic());
    let redacted = report.hunks.iter().all(|hunk| hunk.old_text.is_none());
    if has_semantic && redacted {
        let _ = writeln!(
            out,
            "\nText is redacted. Spans are byte ranges in the original files; pass --explicit to show the changed text."
        );
    }
    out
}

fn render_refusal(report: &Report, refusal: &Refusal) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "REDLINE ERROR ({})\n\n{}\n",
        refusal.code,
        compared(report)
    );
    let _ = writeln!(out, "Cannot produce a verdict.");
    let _ = writeln!(out, "Reason ({}): {}.", refusal.code, refusal.message);
    let _ = writeln!(out, "Detail: {}", refusal.detail);
    let next = refusal.next_command.clone().unwrap_or_else(|| {
        match refusal.code {
            "E_IO" => "check the file path and permissions, then rerun",
            "E_EMPTY_INPUT" => "pass a file that has content",
            "E_BINARY_INPUT" => "extract the text first, then run redline on the extracted text",
            "E_MODE_UNSUPPORTED" => "rerun with --mode auto, raw, whitespace or html-text",
            _ => "redline robot-docs guide",
        }
        .to_owned()
    });
    let _ = writeln!(out, "Next: {next}");
    out
}
