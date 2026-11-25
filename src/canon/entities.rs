//! HTML character references, decoded per the WHATWG standard.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Longest named reference in the standard, without `&` and `;`.
const MAX_NAME_LEN: usize = 32;

fn named() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        ::entities::ENTITIES
            .iter()
            .map(|entity| (entity.entity, entity.characters))
            .collect()
    })
}

/// Decode the character reference starting at `source[pos]`, which must be `&`.
///
/// Returns the decoded text and the offset just past the reference, or `None`
/// when the ampersand is literal text. Never reads at or past `limit`.
pub(super) fn decode(source: &str, pos: usize, limit: usize) -> Option<(String, usize)> {
    let bytes = &source.as_bytes()[..limit];
    if bytes.get(pos) != Some(&b'&') {
        return None;
    }
    match bytes.get(pos + 1) {
        Some(b'#') => decode_numeric(bytes, pos),
        Some(byte) if byte.is_ascii_alphabetic() => decode_named(source, bytes, pos),
        _ => None,
    }
}

fn decode_numeric(bytes: &[u8], pos: usize) -> Option<(String, usize)> {
    let mut cursor = pos + 2;
    let hex = matches!(bytes.get(cursor), Some(b'x' | b'X'));
    if hex {
        cursor += 1;
    }
    let digits_start = cursor;
    let mut value: u32 = 0;
    while let Some(&byte) = bytes.get(cursor) {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' if hex => u32::from(byte - b'a') + 10,
            b'A'..=b'F' if hex => u32::from(byte - b'A') + 10,
            _ => break,
        };
        // Saturate: anything this large is out of range and becomes U+FFFD.
        value = value
            .saturating_mul(if hex { 16 } else { 10 })
            .saturating_add(digit);
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    if bytes.get(cursor) == Some(&b';') {
        cursor += 1;
    }
    Some((numeric_char(value).to_string(), cursor))
}

/// Map a numeric reference to a character, with the standard's replacements.
fn numeric_char(value: u32) -> char {
    /// Windows-1252 readings of the C1 control range 0x80..=0x9F.
    const C1: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    let mapped = match value {
        0 => 0xFFFD,
        0x80..=0x9F => C1[(value - 0x80) as usize],
        other => other,
    };
    char::from_u32(mapped).unwrap_or('\u{fffd}')
}

fn decode_named(source: &str, bytes: &[u8], pos: usize) -> Option<(String, usize)> {
    let name_start = pos + 1;
    let mut name_end = name_start;
    while name_end - name_start < MAX_NAME_LEN
        && bytes.get(name_end).is_some_and(u8::is_ascii_alphanumeric)
    {
        name_end += 1;
    }
    let table = named();
    if bytes.get(name_end) == Some(&b';')
        && let Some(decoded) = table.get(&source[pos..=name_end])
    {
        return Some(((*decoded).to_owned(), name_end + 1));
    }
    // Legacy references such as `&amp` and `&copy` work without a semicolon.
    // The longest matching prefix wins.
    (name_start + 1..=name_end).rev().find_map(|end| {
        table
            .get(&source[pos..end])
            .map(|decoded| ((*decoded).to_owned(), end))
    })
}
