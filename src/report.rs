//! The `redline.report.v1` report.
//!
//! Field order is the declaration order below and is part of the contract.
//! No map types appear here, so serialization is byte-stable.

use serde::Serialize;

use crate::canon::Mode;
use crate::classify::HunkClass;
use crate::refusal::Refusal;

pub const REPORT_VERSION: &str = "redline.report.v1";
pub const TOOL: &str = "redline";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Outcome {
    /// Identical bytes, or only `formatting_only` differences.
    #[serde(rename = "NO_SEMANTIC_CHANGE")]
    NoSemanticChange,
    /// At least one semantic hunk.
    #[serde(rename = "CHANGES_FOUND")]
    ChangesFound,
    #[serde(rename = "REFUSAL")]
    Refusal,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::NoSemanticChange => "NO_SEMANTIC_CHANGE",
            Outcome::ChangesFound => "CHANGES_FOUND",
            Outcome::Refusal => "REFUSAL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub version: &'static str,
    pub tool: &'static str,
    pub outcome: Outcome,
    pub mode: Option<Mode>,
    pub inputs: Inputs,
    pub summary: Option<Summary>,
    pub hunks: Vec<Hunk>,
    pub refusal: Option<Refusal>,
}

impl Report {
    /// The report for a run that produced no verdict.
    pub fn refused(inputs: Inputs, refusal: Refusal) -> Self {
        Report {
            version: REPORT_VERSION,
            tool: TOOL,
            outcome: Outcome::Refusal,
            mode: None,
            inputs,
            summary: None,
            hunks: Vec::new(),
            refusal: Some(refusal),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inputs {
    pub old: InputInfo,
    pub new: InputInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputInfo {
    /// As given by the caller. `None` for in-memory inputs.
    pub path: Option<String>,
    pub bytes: Option<u64>,
    /// `blake3:<hex>` of the whole input.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub total_hunks: usize,
    pub semantic_hunks: usize,
    pub formatting_only_hunks: usize,
    pub numeric_changed_hunks: usize,
    pub clause_added_hunks: usize,
    pub clause_removed_hunks: usize,
    pub text_modified_hunks: usize,
    /// Word and punctuation tokens in each input. Whitespace is not counted.
    pub old_tokens: usize,
    pub new_tokens: usize,
    /// Tokens inside semantic hunks, both sides, over all tokens, both sides.
    pub churn_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hunk {
    pub id: String,
    pub class: HunkClass,
    /// The hunk adds, removes or alters a number.
    pub numeric: bool,
    /// Half-open raw byte range `[start, end)` in the old input. Empty for a
    /// pure insertion, where it marks the position the new text lands at.
    pub old_span: [usize; 2],
    /// Half-open raw byte range `[start, end)` in the new input.
    pub new_span: [usize; 2],
    /// Keyed digest of the old span's raw bytes. See `hunk_digest`.
    pub old_digest: String,
    pub new_digest: String,
    pub old_tokens: usize,
    pub new_tokens: usize,
    /// Canonical text of the old side. Present only in explicit mode, and
    /// only for semantic hunks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    /// An excerpt above was cut at the explicit-mode size limit.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_truncated: bool,
}
