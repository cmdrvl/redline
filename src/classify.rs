//! Deterministic hunk classification.

use std::ops::Range;

use serde::Serialize;

use crate::tokens::{TokenDoc, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HunkClass {
    /// Raw bytes differ, canonical text is identical.
    FormattingOnly,
    /// One or more whole sentences or blocks exist only in the new input.
    ClauseAdded,
    /// One or more whole sentences or blocks exist only in the old input.
    ClauseRemoved,
    /// An edit inside a sentence that changes a number.
    NumericChanged,
    /// Any other edit inside a sentence.
    TextModified,
}

impl HunkClass {
    pub fn is_semantic(self) -> bool {
        self != HunkClass::FormattingOnly
    }

    pub fn as_str(self) -> &'static str {
        match self {
            HunkClass::FormattingOnly => "formatting_only",
            HunkClass::ClauseAdded => "clause_added",
            HunkClass::ClauseRemoved => "clause_removed",
            HunkClass::NumericChanged => "numeric_changed",
            HunkClass::TextModified => "text_modified",
        }
    }
}

/// True when `range` covers whole sentences: it starts at a sentence start
/// and the next text after it starts a new sentence or block, or is the end.
pub fn is_whole_clause(doc: &TokenDoc, range: Range<usize>) -> bool {
    let tokens = &doc.tokens;
    let Some(first) = range
        .clone()
        .find(|&index| tokens[index].kind != TokenKind::Space)
    else {
        return false;
    };
    if !tokens[first].sentence_start {
        return false;
    }
    match (range.end..tokens.len()).find(|&index| tokens[index].kind != TokenKind::Space) {
        Some(next) => tokens[next].sentence_start,
        None => true,
    }
}

/// Ids of the digit-bearing tokens in `range`, in order.
fn numeric_ids(doc: &TokenDoc, range: Range<usize>) -> Vec<u32> {
    doc.tokens[range]
        .iter()
        .filter(|token| token.numeric)
        .map(|token| token.id)
        .collect()
}

/// True when `range` is next to a number and holds nothing but words and
/// marks that change what a number means: `-5`, `(5)`, `March 1`,
/// `$5 billion`. An empty range counts; the other side supplies the edit.
fn modifies_adjacent_number(doc: &TokenDoc, range: Range<usize>) -> bool {
    let tokens = &doc.tokens;
    let text = |index: &usize| tokens[*index].kind != TokenKind::Space;
    if !range
        .clone()
        .filter(text)
        .all(|index| tokens[index].number_modifier)
    {
        return false;
    }
    let before = (0..range.start).rev().find(text);
    let after = (range.end..tokens.len()).find(text);
    before.is_some_and(|index| tokens[index].numeric)
        || after.is_some_and(|index| tokens[index].numeric)
}

/// Classify one change between token ranges. Returns the class and whether
/// the change alters any number.
///
/// A change alters a number when the sequence of digit-bearing tokens
/// differs, or when it only edits the sign, unit, month or magnitude word
/// attached to a number.
///
/// Precedence: a change that adds or removes whole sentences is a clause
/// change even when it contains numbers; `numeric` reports those. Otherwise a
/// change that alters a number is `numeric_changed`, and everything else is
/// `text_modified`.
pub fn classify(
    old: &TokenDoc,
    old_range: Range<usize>,
    new: &TokenDoc,
    new_range: Range<usize>,
) -> (HunkClass, bool) {
    // A change that is only spacing has no modifier in it on either side.
    let has_text = |doc: &TokenDoc, range: Range<usize>| {
        doc.tokens[range]
            .iter()
            .any(|token| token.kind != TokenKind::Space)
    };
    let numeric = numeric_ids(old, old_range.clone()) != numeric_ids(new, new_range.clone())
        || ((has_text(old, old_range.clone()) || has_text(new, new_range.clone()))
            && modifies_adjacent_number(old, old_range.clone())
            && modifies_adjacent_number(new, new_range.clone()));
    let class = if old_range.is_empty() && is_whole_clause(new, new_range.clone()) {
        HunkClass::ClauseAdded
    } else if new_range.is_empty() && is_whole_clause(old, old_range) {
        HunkClass::ClauseRemoved
    } else if numeric {
        HunkClass::NumericChanged
    } else {
        HunkClass::TextModified
    };
    (class, numeric)
}
