//! Tokens, blocks and sentence boundaries over canonical text.

use std::collections::HashMap;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::canon::{CanonicalDoc, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Whitespace. In every mode but `raw` this is exactly one space.
    Space,
    /// Contains at least one letter or digit.
    Word,
    /// Punctuation and symbols.
    Punct,
}

#[derive(Debug, Clone)]
pub struct Token {
    /// Interned text. Equal ids mean equal canonical text, across both inputs.
    pub id: u32,
    pub kind: TokenKind,
    /// Contains a digit.
    pub numeric: bool,
    /// Changes what a neighbouring number means without containing a digit:
    /// a sign, a currency or percent mark, a month, a magnitude word.
    pub number_modifier: bool,
    /// First token of a sentence or block.
    pub sentence_start: bool,
    /// Last token before a block boundary (a line in `raw` mode).
    pub block_end: bool,
    pub canon: Range<usize>,
    /// Raw byte span `[start, end)` in the original input.
    pub raw: Range<usize>,
}

/// A run of tokens between block boundaries.
#[derive(Debug, Clone)]
pub struct Block {
    /// All tokens, including the separator that ends the block.
    pub tokens: Range<usize>,
    /// Tokens that carry the block's text. Excludes the ending separator in
    /// every mode but `raw`, where the newline is content.
    pub content: Range<usize>,
}

#[derive(Debug)]
pub struct TokenDoc {
    pub tokens: Vec<Token>,
    pub blocks: Vec<Block>,
    /// `raw` mode: blocks are lines and the newline ending one is content.
    pub line_mode: bool,
}

/// Words and marks that change the meaning of the number next to them.
const NUMBER_MODIFIERS: &[&str] = &[
    "-",
    "+",
    "(",
    ")",
    "$",
    "%",
    "\u{2212}",
    "\u{2013}",
    "\u{20ac}",
    "\u{a3}",
    "\u{a5}",
    "hundred",
    "thousand",
    "million",
    "billion",
    "trillion",
    "percent",
    "bps",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "jan",
    "feb",
    "mar",
    "apr",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
];

fn is_number_modifier(piece: &str) -> bool {
    piece.len() <= 9 && NUMBER_MODIFIERS.contains(&piece.to_ascii_lowercase().as_str())
}

impl TokenDoc {
    /// Number of tokens that are not whitespace.
    pub fn count_words(&self, range: Range<usize>) -> usize {
        self.tokens[range]
            .iter()
            .filter(|token| token.kind != TokenKind::Space)
            .count()
    }
}

/// Assigns one id per distinct token text, shared by both inputs.
#[derive(Default)]
pub struct Interner<'a> {
    ids: HashMap<&'a str, u32>,
}

impl<'a> Interner<'a> {
    fn intern(&mut self, text: &'a str) -> u32 {
        let next = self.ids.len() as u32;
        *self.ids.entry(text).or_insert(next)
    }
}

/// Split `doc` into tokens and blocks.
pub fn tokenize<'a>(doc: &'a CanonicalDoc, mode: Mode, interner: &mut Interner<'a>) -> TokenDoc {
    let mut tokens: Vec<Token> = Vec::new();
    let mut breaks = doc.block_breaks.iter().copied().peekable();

    for (start, piece) in doc.text.split_word_bound_indices() {
        let canon = start..start + piece.len();
        let kind = if piece.chars().all(char::is_whitespace) {
            TokenKind::Space
        } else if piece.chars().any(char::is_alphanumeric) {
            TokenKind::Word
        } else {
            TokenKind::Punct
        };
        let block_end = match mode {
            Mode::Raw => piece.contains('\n'),
            _ => {
                while breaks.peek().is_some_and(|&at| at < start) {
                    breaks.next();
                }
                breaks.peek() == Some(&start)
            }
        };
        tokens.push(Token {
            id: interner.intern(piece),
            kind,
            numeric: piece.chars().any(char::is_numeric),
            number_modifier: is_number_modifier(piece),
            sentence_start: false,
            block_end,
            raw: doc.raw_span(canon.clone()),
            canon,
        });
    }

    let mut blocks = Vec::new();
    let mut block_start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if token.block_end {
            let content_end = if mode == Mode::Raw { index + 1 } else { index };
            blocks.push(Block {
                tokens: block_start..index + 1,
                content: block_start..content_end,
            });
            block_start = index + 1;
        }
    }
    if block_start < tokens.len() {
        blocks.push(Block {
            tokens: block_start..tokens.len(),
            content: block_start..tokens.len(),
        });
    }

    mark_sentence_starts(doc, &mut tokens, &blocks);
    TokenDoc {
        tokens,
        blocks,
        line_mode: mode == Mode::Raw,
    }
}

/// Flag the first non-space token of every sentence, block by block.
fn mark_sentence_starts(doc: &CanonicalDoc, tokens: &mut [Token], blocks: &[Block]) {
    for block in blocks {
        if block.content.is_empty() {
            continue;
        }
        let text_start = tokens[block.content.start].canon.start;
        let text_end = tokens[block.content.end - 1].canon.end;
        let mut cursor = block.content.start;
        for (offset, _) in doc.text[text_start..text_end].split_sentence_bound_indices() {
            let sentence_start = text_start + offset;
            while cursor < block.content.end
                && (tokens[cursor].canon.start < sentence_start
                    || tokens[cursor].kind == TokenKind::Space)
            {
                cursor += 1;
            }
            if cursor < block.content.end {
                tokens[cursor].sentence_start = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::{Edges, canonicalize};

    fn words(source: &str, mode: Mode) -> Vec<String> {
        let doc = canonicalize(source, mode, Edges::Document);
        let mut interner = Interner::default();
        let tokenized = tokenize(&doc, mode, &mut interner);
        tokenized
            .tokens
            .iter()
            .map(|token| doc.text[token.canon.clone()].to_owned())
            .collect()
    }

    #[test]
    fn numbers_stay_whole() {
        assert_eq!(
            words("pay $1,250,000.00 at 5.25%", Mode::Whitespace),
            ["pay", " ", "$", "1,250,000.00", " ", "at", " ", "5.25", "%"]
        );
    }

    #[test]
    fn sentence_and_block_flags() {
        let source = "First one. Second one.\n\nNew block here.";
        let doc = canonicalize(source, Mode::Whitespace, Edges::Document);
        let mut interner = Interner::default();
        let tokenized = tokenize(&doc, Mode::Whitespace, &mut interner);
        let starts: Vec<&str> = tokenized
            .tokens
            .iter()
            .filter(|token| token.sentence_start)
            .map(|token| &doc.text[token.canon.clone()])
            .collect();
        assert_eq!(starts, ["First", "Second", "New"]);
        assert_eq!(tokenized.blocks.len(), 2);
        assert_eq!(
            tokenized.blocks[0].content.len() + 1,
            tokenized.blocks[0].tokens.len()
        );
    }

    #[test]
    fn raw_mode_blocks_are_lines() {
        let doc = canonicalize("a\nb\n", Mode::Raw, Edges::Document);
        let mut interner = Interner::default();
        let tokenized = tokenize(&doc, Mode::Raw, &mut interner);
        assert_eq!(tokenized.blocks.len(), 2);
        assert_eq!(tokenized.blocks[0].content, 0..2);
    }

    #[test]
    fn equal_text_shares_an_id_across_documents() {
        let old = canonicalize("alpha beta", Mode::Whitespace, Edges::Document);
        let new = canonicalize("beta alpha", Mode::Whitespace, Edges::Document);
        let mut interner = Interner::default();
        let a = tokenize(&old, Mode::Whitespace, &mut interner);
        let b = tokenize(&new, Mode::Whitespace, &mut interner);
        assert_eq!(a.tokens[0].id, b.tokens[2].id);
        assert_ne!(a.tokens[0].id, a.tokens[2].id);
    }
}
