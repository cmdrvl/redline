//! Two-level deterministic diff: blocks first, then words.
//!
//! Matching whole blocks first anchors the alignment on text that is
//! certainly unchanged, so the word-level diff only ever sees the regions
//! between anchors. That keeps large documents fast and stops common words
//! from being matched across unrelated paragraphs.

use std::collections::HashMap;
use std::ops::Range;

use imara_diff::{Algorithm, Diff, Token as DiffToken};

use crate::classify::is_whole_clause;
use crate::tokens::{TokenDoc, TokenKind};

/// A changed region, as token index ranges into each input. Either side may
/// be empty (a pure insertion or removal), never both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// A pure insertion or removal may be slid this many tokens while looking
/// for a position that lines up with sentence boundaries.
const MAX_SLIDE: u32 = 256;

/// Two changes separated by at most this many unchanged tokens, none of them
/// a word, are reported as one change.
const MAX_MERGE_GAP: usize = 4;

/// All changes between `old` and `new`, in document order.
pub fn diff(old: &TokenDoc, new: &TokenDoc) -> Vec<Change> {
    let old_ids: Vec<u32> = old.tokens.iter().map(|token| token.id).collect();
    let new_ids: Vec<u32> = new.tokens.iter().map(|token| token.id).collect();
    let sides = Sides {
        old,
        new,
        old_ids: &old_ids,
        new_ids: &new_ids,
    };

    let mut changes = Vec::new();
    // Each region runs from the end of one matched block to the start of the
    // next. `context` is the length of the matched block before it: unchanged
    // text an insertion may slide back into.
    let mut old_cursor = 0;
    let mut new_cursor = 0;
    let mut context = 0;
    for (old_block, new_block) in matched_blocks(&sides) {
        let old_content = old.blocks[old_block].content.clone();
        let new_content = new.blocks[new_block].content.clone();
        diff_region(
            &sides,
            context,
            old_cursor..old_content.start,
            new_cursor..new_content.start,
            &mut changes,
        );
        old_cursor = old_content.end;
        new_cursor = new_content.end;
        context = old_content.len();
    }
    diff_region(
        &sides,
        context,
        old_cursor..old_ids.len(),
        new_cursor..new_ids.len(),
        &mut changes,
    );

    merge_adjacent(old, new, changes)
}

struct Sides<'a> {
    old: &'a TokenDoc,
    new: &'a TokenDoc,
    old_ids: &'a [u32],
    new_ids: &'a [u32],
}

/// Pairs of block indices whose content is identical, in order.
fn matched_blocks(sides: &Sides<'_>) -> Vec<(usize, usize)> {
    let mut keys: HashMap<&[u32], u32> = HashMap::new();
    let mut before = Vec::with_capacity(sides.old.blocks.len());
    for block in &sides.old.blocks {
        let next = keys.len() as u32;
        let key = keys
            .entry(&sides.old_ids[block.content.clone()])
            .or_insert(next);
        before.push(DiffToken(*key));
    }
    let mut after = Vec::with_capacity(sides.new.blocks.len());
    for block in &sides.new.blocks {
        let next = keys.len() as u32;
        let key = keys
            .entry(&sides.new_ids[block.content.clone()])
            .or_insert(next);
        after.push(DiffToken(*key));
    }

    let mut diff = Diff::default();
    diff.compute_with(Algorithm::Histogram, &before, &after, keys.len() as u32);

    let mut pairs = Vec::new();
    let mut old_index = 0usize;
    let mut new_index = 0usize;
    for hunk in diff.hunks() {
        while old_index < hunk.before.start as usize {
            pairs.push((old_index, new_index));
            old_index += 1;
            new_index += 1;
        }
        old_index = hunk.before.end as usize;
        new_index = hunk.after.end as usize;
    }
    while old_index < before.len() && new_index < after.len() {
        pairs.push((old_index, new_index));
        old_index += 1;
        new_index += 1;
    }
    pairs
}

/// Regions with more tokens than this, both sides together, are split on
/// unique words before the word diff runs.
const ANCHOR_THRESHOLD: usize = 4096;

/// A piece with about this many tokens on each side that still has no unique
/// word in common is reported as one replacement. The word diff is quadratic
/// on such input (the only tokens the sides share are spaces and very common
/// words), and an alignment of two long texts with no distinctive word in
/// common would mean nothing anyway.
const REPLACE_THRESHOLD: usize = 4096;

/// Word-level diff of one region between matched blocks.
///
/// The `context` tokens before each range are known to be equal on both
/// sides. They take part in the diff so that an insertion or removal at the
/// start of the region can slide back onto a sentence boundary.
///
/// A small region goes straight to the word diff. A large one is first cut
/// at words that occur exactly once on each side (the patience method), which
/// keeps the cost near linear however much of the region changed.
fn diff_region(
    sides: &Sides<'_>,
    context: usize,
    old_range: Range<usize>,
    new_range: Range<usize>,
    changes: &mut Vec<Change>,
) {
    if sides.old_ids[old_range.clone()] == sides.new_ids[new_range.clone()] {
        return;
    }
    if old_range.len() + new_range.len() <= ANCHOR_THRESHOLD {
        diff_tokens(sides, context, old_range, new_range, changes);
        return;
    }

    // Tokens known equal immediately before the current piece.
    let mut run = context;
    let mut old_cursor = old_range.start;
    let mut new_cursor = new_range.start;
    let ends = (old_range.end, new_range.end);
    for (old_anchor, new_anchor) in unique_anchors(sides, old_range, new_range)
        .into_iter()
        .chain([ends])
    {
        let old_piece = old_cursor..old_anchor;
        let new_piece = new_cursor..new_anchor;
        if sides.old_ids[old_piece.clone()] == sides.new_ids[new_piece.clone()] {
            run += old_piece.len() + 1;
        } else {
            diff_piece(
                sides,
                run.min(MAX_SLIDE as usize),
                old_piece,
                new_piece,
                changes,
            );
            run = 1;
        }
        old_cursor = old_anchor + 1;
        new_cursor = new_anchor + 1;
    }
}

/// Positions of the words that occur exactly once in each range, as pairs in
/// increasing order on both sides: the longest such chain.
fn unique_anchors(
    sides: &Sides<'_>,
    old_range: Range<usize>,
    new_range: Range<usize>,
) -> Vec<(usize, usize)> {
    // Per token id: occurrences and last position, on each side.
    let mut seen: HashMap<u32, (u32, usize, u32, usize)> = HashMap::new();
    for index in old_range.clone() {
        if sides.old.tokens[index].kind == TokenKind::Word {
            let entry = seen.entry(sides.old_ids[index]).or_default();
            entry.0 += 1;
            entry.1 = index;
        }
    }
    for index in new_range {
        if sides.new.tokens[index].kind == TokenKind::Word {
            let entry = seen.entry(sides.new_ids[index]).or_default();
            entry.2 += 1;
            entry.3 = index;
        }
    }
    // Walk the old side in order, so the result does not depend on the
    // iteration order of the map.
    let pairs: Vec<(usize, usize)> = old_range
        .filter_map(|index| match seen.get(&sides.old_ids[index]) {
            Some(&(1, old_at, 1, new_at)) if old_at == index => Some((old_at, new_at)),
            _ => None,
        })
        .collect();

    // Longest increasing subsequence on the new position (patience sorting).
    // `tails[k]` is the pair ending the best chain of length k + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; pairs.len()];
    for (at, pair) in pairs.iter().enumerate() {
        let slot = tails.partition_point(|&tail| pairs[tail].1 < pair.1);
        previous[at] = slot.checked_sub(1).map(|before| tails[before]);
        if slot == tails.len() {
            tails.push(at);
        } else {
            tails[slot] = at;
        }
    }
    let mut chain = Vec::with_capacity(tails.len());
    let mut cursor = tails.last().copied();
    while let Some(at) = cursor {
        chain.push(pairs[at]);
        cursor = previous[at];
    }
    chain.reverse();
    chain
}

/// Diff one piece between unique-word anchors.
fn diff_piece(
    sides: &Sides<'_>,
    context: usize,
    old_range: Range<usize>,
    new_range: Range<usize>,
    changes: &mut Vec<Change>,
) {
    // The word diff costs about the product of the two lengths in the worst
    // case, so a long side against a short one is still cheap.
    if old_range.len().saturating_mul(new_range.len()) <= REPLACE_THRESHOLD * REPLACE_THRESHOLD {
        diff_tokens(sides, context, old_range, new_range, changes);
        return;
    }
    // One replacement, less whatever the two sides share at each end.
    let old_ids = &sides.old_ids[old_range.clone()];
    let new_ids = &sides.new_ids[new_range.clone()];
    let prefix = old_ids
        .iter()
        .zip(new_ids)
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = old_ids[prefix..]
        .iter()
        .rev()
        .zip(new_ids[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    changes.push(Change {
        old: old_range.start + prefix..old_range.end - suffix,
        new: new_range.start + prefix..new_range.end - suffix,
    });
}

/// The word diff itself, over one bounded stretch of tokens.
fn diff_tokens(
    sides: &Sides<'_>,
    context: usize,
    old_range: Range<usize>,
    new_range: Range<usize>,
    changes: &mut Vec<Change>,
) {
    let old_start = old_range.start - context;
    let new_start = new_range.start - context;

    // Histogram diff allocates per distinct token, so renumber densely.
    let mut local: HashMap<u32, u32> = HashMap::new();
    let mut renumber = |id: u32| -> DiffToken {
        let next = local.len() as u32;
        DiffToken(*local.entry(id).or_insert(next))
    };
    let before: Vec<DiffToken> = sides.old_ids[old_start..old_range.end]
        .iter()
        .map(|&id| renumber(id))
        .collect();
    let after: Vec<DiffToken> = sides.new_ids[new_start..new_range.end]
        .iter()
        .map(|&id| renumber(id))
        .collect();
    let token_count = local.len() as u32;

    let mut diff = Diff::default();
    diff.compute_with(Algorithm::Histogram, &before, &after, token_count);

    // The postprocessor calls this for pure insertions (over `after`) and pure
    // removals (over `before`) that can slide. It passes the token slice it is
    // working on, which tells the two sides apart. Of the legal positions, take
    // the latest one that covers whole sentences; failing that, the latest.
    let after_ptr = after.as_ptr();
    let heuristic = |tokens: &[DiffToken], hunk: Range<u32>, earliest_end: u32| -> u32 {
        let (doc, base) = if std::ptr::eq(tokens.as_ptr(), after_ptr) {
            (sides.new, new_start)
        } else {
            (sides.old, old_start)
        };
        let size = hunk.end - hunk.start;
        let lowest = earliest_end
            .max(size)
            .max(hunk.end.saturating_sub(MAX_SLIDE));
        let covers_clause = |end: &u32| {
            let start = end - size;
            is_whole_clause(doc, base + start as usize..base + *end as usize)
        };
        // A sentence added at the end of a paragraph can be read as taking
        // the space before it or the paragraph break after it. Prefer the
        // reading that leaves the paragraph break alone. In line mode the
        // newline belongs to its line, so there the preference is reversed:
        // an added line is the line and its newline.
        let keeps_block_break =
            |end: &u32| doc.tokens[base + *end as usize - 1].block_end == doc.line_mode;
        (lowest..=hunk.end)
            .rev()
            .find(|end| covers_clause(end) && keeps_block_break(end))
            .or_else(|| (lowest..=hunk.end).rev().find(covers_clause))
            .unwrap_or(hunk.end)
    };
    diff.postprocess_with(&before, &after, heuristic);

    for hunk in diff.hunks() {
        changes.push(Change {
            old: old_start + hunk.before.start as usize..old_start + hunk.before.end as usize,
            new: new_start + hunk.after.start as usize..new_start + hunk.after.end as usize,
        });
    }
}

/// Join changes that a reader would see as one edit.
///
/// Two rules, both confined to a single block:
///
/// 1. Changes separated only by a little punctuation or spacing are one
///    change, so `March 1, 2024` -> `April 15, 2025` is not three.
/// 2. An unchanged run that is no longer than the edits on both sides of it
///    is a coincidence, not an anchor. When a paragraph is rewritten, the
///    diff still matches stray words such as `the` and `of`; folding those in
///    reports the rewrite as one change in place of dozens of fragments.
///
/// A whole added or removed sentence is never folded into a neighbour.
fn merge_adjacent(old: &TokenDoc, new: &TokenDoc, changes: Vec<Change>) -> Vec<Change> {
    let words = |doc: &TokenDoc, range: Range<usize>| {
        doc.tokens[range]
            .iter()
            .filter(|token| token.kind == TokenKind::Word)
            .count()
    };
    let is_clause = |change: &Change| {
        (change.old.is_empty() && is_whole_clause(new, change.new.clone()))
            || (change.new.is_empty() && is_whole_clause(old, change.old.clone()))
    };

    let mut merged: Vec<Change> = Vec::with_capacity(changes.len());
    // Word counts of the last merged change on each side, kept as a running
    // total. Recounting it would make a fully rewritten document quadratic.
    let mut last_words = (0usize, 0usize);
    for change in changes {
        let change_words = (
            words(old, change.old.clone()),
            words(new, change.new.clone()),
        );
        if let Some(last) = merged.last_mut() {
            let old_gap = last.old.end..change.old.start;
            let new_gap = last.new.end..change.new.start;
            let same_block = old.tokens[old_gap.clone()]
                .iter()
                .chain(&new.tokens[new_gap])
                .all(|token| !token.block_end);
            let gap_words = words(old, old_gap.clone());
            let punctuation_only = gap_words == 0 && old_gap.len() <= MAX_MERGE_GAP;
            let coincidence = gap_words <= last_words.0.max(last_words.1)
                && gap_words <= change_words.0.max(change_words.1);
            let joinable = same_block
                && (punctuation_only || coincidence)
                && !is_clause(last)
                && !is_clause(&change);
            if joinable {
                last.old.end = change.old.end;
                last.new.end = change.new.end;
                last_words.0 += gap_words + change_words.0;
                last_words.1 += gap_words + change_words.1;
                continue;
            }
        }
        last_words = change_words;
        merged.push(change);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::{Edges, Mode, canonicalize};
    use crate::tokens::{Interner, tokenize};

    /// Changed text on each side, for whitespace-mode inputs.
    fn changed(old: &str, new: &str) -> Vec<(String, String)> {
        let old_doc = canonicalize(old, Mode::Whitespace, Edges::Document);
        let new_doc = canonicalize(new, Mode::Whitespace, Edges::Document);
        let mut interner = Interner::default();
        let old_tokens = tokenize(&old_doc, Mode::Whitespace, &mut interner);
        let new_tokens = tokenize(&new_doc, Mode::Whitespace, &mut interner);
        let text = |doc: &crate::canon::CanonicalDoc, tokens: &TokenDoc, range: Range<usize>| {
            if range.is_empty() {
                return String::new();
            }
            let start = tokens.tokens[range.start].canon.start;
            let end = tokens.tokens[range.end - 1].canon.end;
            doc.text[start..end].to_owned()
        };
        diff(&old_tokens, &new_tokens)
            .into_iter()
            .map(|change| {
                (
                    text(&old_doc, &old_tokens, change.old),
                    text(&new_doc, &new_tokens, change.new),
                )
            })
            .collect()
    }

    fn pair(old: &str, new: &str) -> (String, String) {
        (old.to_owned(), new.to_owned())
    }

    #[test]
    fn identical_inputs_have_no_changes() {
        assert!(changed("a b c", "a  b\nc").is_empty());
    }

    #[test]
    fn single_word_replacement() {
        assert_eq!(
            changed("the rate is 5.25% today", "the rate is 5.50% today"),
            [pair("5.25", "5.50")]
        );
    }

    #[test]
    fn date_edit_is_one_change() {
        assert_eq!(
            changed("due March 1, 2024 in full", "due April 15, 2025 in full"),
            [pair("March 1, 2024", "April 15, 2025")]
        );
    }

    #[test]
    fn inserted_sentence_lines_up_with_sentence_boundaries() {
        let got = changed(
            "The cat sat. The dog ran.",
            "The cat sat. The bird flew. The dog ran.",
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "");
        assert_eq!(got[0].1.trim(), "The bird flew.");
    }

    #[test]
    fn inserted_paragraph_between_unchanged_paragraphs() {
        let got = changed(
            "Alpha one.\n\nGamma three.",
            "Alpha one.\n\nBeta two.\n\nGamma three.",
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "");
        assert_eq!(got[0].1.trim(), "Beta two.");
    }

    #[test]
    fn spacing_change_that_alters_text_is_a_change() {
        assert_eq!(changed("can not", "cannot"), [pair("can not", "cannot")]);
        assert_eq!(changed("a , b", "a, b"), [pair(" ", "")]);
    }

    #[test]
    fn changes_far_apart_stay_separate() {
        let got = changed("one two three four five six", "ONE two three four five SIX");
        assert_eq!(got, [pair("one", "ONE"), pair("six", "SIX")]);
    }
}
