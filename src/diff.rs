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
        before.push(*key);
    }
    let mut after = Vec::with_capacity(sides.new.blocks.len());
    for block in &sides.new.blocks {
        let next = keys.len() as u32;
        let key = keys
            .entry(&sides.new_ids[block.content.clone()])
            .or_insert(next);
        after.push(*key);
    }

    let mut pairs = Vec::new();
    if is_cheap(before.len(), after.len()) {
        push_equal_pairs(
            &histogram_edits(&before, &after),
            0..before.len(),
            0,
            &mut pairs,
        );
        return pairs;
    }

    // Many blocks: cut at blocks that occur once on each side, then match
    // inside each piece. A long piece of repeated values (a column of `yes`
    // and `no` cells) is matched only if few of its blocks changed; if not,
    // it is left to the word diff, which reports it coarsely.
    let mut old_cursor = 0;
    let mut new_cursor = 0;
    let ends = (before.len(), after.len());
    for (old_anchor, new_anchor) in unique_anchors(&before, &after, |_| true, |_| true)
        .into_iter()
        .chain([ends])
    {
        let old_piece = &before[old_cursor..old_anchor];
        let new_piece = &after[new_cursor..new_anchor];
        let edits = if is_cheap(old_piece.len(), new_piece.len()) {
            Some(histogram_edits(old_piece, new_piece))
        } else {
            bounded_myers(old_piece, new_piece)
        };
        if let Some(edits) = edits {
            push_equal_pairs(&edits, old_cursor..old_anchor, new_cursor, &mut pairs);
        }
        if old_anchor < before.len() {
            pairs.push((old_anchor, new_anchor));
        }
        old_cursor = old_anchor + 1;
        new_cursor = new_anchor + 1;
    }
    pairs
}

/// One edit: the range replaced on each side. Either may be empty.
type Edit = (Range<usize>, Range<usize>);

/// True when the two lengths are small enough to hand to the diff library,
/// whose cost grows with their product in the worst case. A long side against
/// a short one is still cheap.
fn is_cheap(old_len: usize, new_len: usize) -> bool {
    old_len.saturating_mul(new_len) <= REPLACE_THRESHOLD * REPLACE_THRESHOLD
}

/// Record the positions `edits` leave equal. `old_range` is the stretch the
/// edits are relative to and `new_start` where it begins on the other side.
fn push_equal_pairs(
    edits: &[Edit],
    old_range: Range<usize>,
    new_start: usize,
    pairs: &mut Vec<(usize, usize)>,
) {
    let mut old_index = old_range.start;
    let mut new_index = new_start;
    for (old_edit, new_edit) in edits {
        while old_index < old_range.start + old_edit.start {
            pairs.push((old_index, new_index));
            old_index += 1;
            new_index += 1;
        }
        old_index = old_range.start + old_edit.end;
        new_index = new_start + new_edit.end;
    }
    while old_index < old_range.end {
        pairs.push((old_index, new_index));
        old_index += 1;
        new_index += 1;
    }
}

/// Histogram diff of two id sequences, as edits.
fn histogram_edits(old: &[u32], new: &[u32]) -> Vec<Edit> {
    // The algorithm allocates per distinct token, so renumber densely.
    let mut local: HashMap<u32, u32> = HashMap::new();
    let mut renumber = |id: &u32| -> DiffToken {
        let next = local.len() as u32;
        DiffToken(*local.entry(*id).or_insert(next))
    };
    let before: Vec<DiffToken> = old.iter().map(&mut renumber).collect();
    let after: Vec<DiffToken> = new.iter().map(&mut renumber).collect();
    let mut diff = Diff::default();
    diff.compute_with(Algorithm::Histogram, &before, &after, local.len() as u32);
    diff.hunks()
        .map(|hunk| {
            (
                hunk.before.start as usize..hunk.before.end as usize,
                hunk.after.start as usize..hunk.after.end as usize,
            )
        })
        .collect()
}

/// Work allowed in one `bounded_myers` call, in token comparisons.
const MYERS_BUDGET: usize = 100_000_000;

/// Greedy Myers diff that gives up when the sequences are too different.
///
/// The diff library's algorithms are quadratic on long sequences made of a
/// few repeated values, whatever the number of edits. This one costs the
/// sequence length times the number of edits, so it handles the common case
/// of a long, repetitive stretch with a handful of changes, and returns
/// `None` once the edit count passes what the budget allows.
fn bounded_myers(old: &[u32], new: &[u32]) -> Option<Vec<Edit>> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 && m == 0 {
        return Some(Vec::new());
    }
    if n == 0 || m == 0 {
        // A pure insertion or removal, however long, is one edit.
        return Some(vec![(prefix..prefix + a.len(), prefix..prefix + b.len())]);
    }

    let max_d = (MYERS_BUDGET / (a.len() + b.len())).clamp(16, 1024) as isize;
    // `reach[offset + k]` is the furthest position on `a` reached on
    // diagonal `k = x - y`. `trace[d]` is the part of it round `d` reads.
    let offset = max_d + 1;
    let mut reach = vec![0isize; (2 * max_d + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut rounds = None;
    'search: for d in 0..=max_d {
        trace.push(reach[(offset - d - 1) as usize..=(offset + d + 1) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let at = (offset + k) as usize;
            let down = k == -d || (k != d && reach[at - 1] < reach[at + 1]);
            let mut x = if down {
                reach[at + 1]
            } else {
                reach[at - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            reach[at] = x;
            if x >= n && y >= m {
                rounds = Some(d);
                break 'search;
            }
            k += 2;
        }
    }

    // Walk back from the end, one edit per round.
    let mut steps: Vec<Edit> = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=rounds?).rev() {
        let before = &trace[d as usize];
        let at = |k: isize| before[(k + d + 1) as usize];
        let k = x - y;
        let down = k == -d || (k != d && at(k - 1) < at(k + 1));
        let previous_k = if down { k + 1 } else { k - 1 };
        let previous_x = at(previous_k);
        let previous_y = previous_x - previous_k;
        let (old_step, new_step) = if down {
            (previous_x..previous_x, previous_y..previous_y + 1)
        } else {
            (previous_x..previous_x + 1, previous_y..previous_y)
        };
        steps.push((
            prefix + old_step.start as usize..prefix + old_step.end as usize,
            prefix + new_step.start as usize..prefix + new_step.end as usize,
        ));
        x = previous_x;
        y = previous_y;
    }
    steps.reverse();

    // Join single steps that touch into whole edits.
    let mut edits: Vec<Edit> = Vec::with_capacity(steps.len());
    for (old_step, new_step) in steps {
        match edits.last_mut() {
            Some((old_edit, new_edit))
                if old_edit.end == old_step.start && new_edit.end == new_step.start =>
            {
                old_edit.end = old_step.end;
                new_edit.end = new_step.end;
            }
            _ => edits.push((old_step, new_step)),
        }
    }
    Some(edits)
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
    let (old_base, new_base) = (old_range.start, new_range.start);
    let is_word = |doc: &TokenDoc, index: usize| doc.tokens[index].kind == TokenKind::Word;
    let anchors = unique_anchors(
        &sides.old_ids[old_range],
        &sides.new_ids[new_range],
        |index| is_word(sides.old, old_base + index),
        |index| is_word(sides.new, new_base + index),
    );
    for (old_anchor, new_anchor) in anchors
        .into_iter()
        .map(|(old_at, new_at)| (old_base + old_at, new_base + new_at))
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
    old: &[u32],
    new: &[u32],
    old_eligible: impl Fn(usize) -> bool,
    new_eligible: impl Fn(usize) -> bool,
) -> Vec<(usize, usize)> {
    // Per id: occurrences and last position, on each side.
    let mut seen: HashMap<u32, (u32, usize, u32, usize)> = HashMap::new();
    for (index, id) in old.iter().enumerate() {
        if old_eligible(index) {
            let entry = seen.entry(*id).or_default();
            entry.0 += 1;
            entry.1 = index;
        }
    }
    for (index, id) in new.iter().enumerate() {
        if new_eligible(index) {
            let entry = seen.entry(*id).or_default();
            entry.2 += 1;
            entry.3 = index;
        }
    }
    // Walk the old side in order, so the result does not depend on the
    // iteration order of the map.
    let pairs: Vec<(usize, usize)> = old
        .iter()
        .enumerate()
        .filter_map(|(index, id)| match seen.get(id) {
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
    if is_cheap(old_range.len(), new_range.len()) {
        diff_tokens(sides, context, old_range, new_range, changes);
        return;
    }
    let old_ids = &sides.old_ids[old_range.clone()];
    let new_ids = &sides.new_ids[new_range.clone()];
    // A long repetitive piece with few edits is still diffed exactly.
    if let Some(edits) = bounded_myers(old_ids, new_ids) {
        changes.extend(edits.into_iter().map(|(old_edit, new_edit)| Change {
            old: old_range.start + old_edit.start..old_range.start + old_edit.end,
            new: new_range.start + new_edit.start..new_range.start + new_edit.end,
        }));
        return;
    }
    // One replacement, less whatever the two sides share at each end.
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

    /// Rebuild `new` from `old` and the edits. Fails if the edits are out of
    /// order, overlap, or claim something equal that is not.
    fn apply(old: &[u32], new: &[u32], edits: &[Edit]) -> Vec<u32> {
        let mut out = Vec::new();
        let mut old_at = 0;
        let mut new_at = 0;
        for (old_edit, new_edit) in edits {
            assert!(
                old_edit.start >= old_at && new_edit.start >= new_at,
                "{edits:?}"
            );
            assert_eq!(
                old_edit.start - old_at,
                new_edit.start - new_at,
                "unequal gap"
            );
            assert!(!(old_edit.is_empty() && new_edit.is_empty()), "empty edit");
            out.extend_from_slice(&old[old_at..old_edit.start]);
            out.extend_from_slice(&new[new_edit.clone()]);
            old_at = old_edit.end;
            new_at = new_edit.end;
        }
        out.extend_from_slice(&old[old_at..]);
        out
    }

    /// Small deterministic generator; the tests need variety, not quality.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound
        }
    }

    #[test]
    fn bounded_myers_edits_rebuild_the_new_sequence() {
        let mut rng = Lcg(7);
        for round in 0..400 {
            let len = rng.next(60) as usize;
            let alphabet = 1 + rng.next(4);
            let old: Vec<u32> = (0..len).map(|_| rng.next(alphabet) as u32).collect();
            let mut new = old.clone();
            for _ in 0..rng.next(6) {
                let at = rng.next(new.len() as u64 + 1) as usize;
                match rng.next(3) {
                    0 => new.insert(at, rng.next(alphabet + 1) as u32),
                    1 if at < new.len() => {
                        new.remove(at);
                    }
                    _ if at < new.len() => new[at] = 9,
                    _ => {}
                }
            }
            let edits = bounded_myers(&old, &new).expect("few edits fit the budget");
            assert_eq!(apply(&old, &new, &edits), new, "round {round}");
            let cost: usize = edits.iter().map(|(o, n)| o.len() + n.len()).sum();
            assert!(
                cost <= 12,
                "round {round}: {cost} steps for at most 6 edits"
            );
        }
    }

    #[test]
    fn bounded_myers_handles_the_edges() {
        assert_eq!(bounded_myers(&[], &[]), Some(vec![]));
        assert_eq!(bounded_myers(&[1, 2], &[1, 2]), Some(vec![]));
        assert_eq!(bounded_myers(&[], &[1, 2]), Some(vec![(0..0, 0..2)]));
        assert_eq!(bounded_myers(&[1, 2, 3], &[1, 3]), Some(vec![(1..2, 1..1)]));
        // A pure insertion of any length is one edit, not thousands of steps.
        let long: Vec<u32> = (0..50_000).map(|i| i % 2).collect();
        assert_eq!(
            bounded_myers(&long[..10], &long),
            Some(vec![(10..10, 10..50_000)])
        );
    }

    #[test]
    fn bounded_myers_gives_up_on_unrelated_sequences() {
        let old: Vec<u32> = (0..20_000).map(|i| i % 2).collect();
        let new: Vec<u32> = (0..20_000).map(|i| 2 + i % 2).collect();
        assert_eq!(bounded_myers(&old, &new), None);
    }

    /// The block matcher's pairs must be equal blocks in increasing order,
    /// on the anchored path as well as the direct one.
    #[test]
    fn matched_blocks_are_equal_and_ordered_for_many_repeated_blocks() {
        let mut rng = Lcg(11);
        let lines: Vec<&str> = (0..9000)
            .map(|_| if rng.next(2) == 0 { "yes" } else { "no" })
            .collect();
        let mut edited = lines.clone();
        edited[100] = "maybe";
        edited[4000] = "unique here";
        edited.insert(7000, "extra");
        edited.remove(8500);
        let old_text = lines.join("\n\n");
        let new_text = edited.join("\n\n");
        let old_doc = canonicalize(&old_text, Mode::Whitespace, Edges::Document);
        let new_doc = canonicalize(&new_text, Mode::Whitespace, Edges::Document);
        let mut interner = Interner::default();
        let old = tokenize(&old_doc, Mode::Whitespace, &mut interner);
        let new = tokenize(&new_doc, Mode::Whitespace, &mut interner);
        let old_ids: Vec<u32> = old.tokens.iter().map(|token| token.id).collect();
        let new_ids: Vec<u32> = new.tokens.iter().map(|token| token.id).collect();
        let sides = Sides {
            old: &old,
            new: &new,
            old_ids: &old_ids,
            new_ids: &new_ids,
        };
        assert!(!is_cheap(old.blocks.len(), new.blocks.len()));
        let pairs = matched_blocks(&sides);
        assert!(pairs.len() >= 8990, "only {} of 9000 matched", pairs.len());
        for window in pairs.windows(2) {
            assert!(window[0].0 < window[1].0 && window[0].1 < window[1].1);
        }
        for (old_block, new_block) in pairs {
            assert_eq!(
                old_ids[old.blocks[old_block].content.clone()],
                new_ids[new.blocks[new_block].content.clone()]
            );
        }
        // Four scattered edits. The replaced block comes back as an added
        // block and a removed one: among identical neighbours the removal can
        // sit on any of them, and the bounded diff does not slide it back.
        assert_eq!(
            changed(&old_text, &new_text),
            [
                pair("yes", "maybe"),
                pair("", " unique here"),
                pair(" no", ""),
                pair("", " extra"),
                pair(" no", ""),
            ]
        );
    }
}
