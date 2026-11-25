# `redline` — Design Specification (Pre-Implementation)

## 1. Problem Statement

Agents and analysts constantly need to know **what actually changed** between two versions of an unstructured or semi-structured document:

- SEC filing amendments (`8-K` -> `8-K/A`, `S-1` -> `S-1/A` -> `424B4`, `13D` -> `13D/A`, `10-K` -> `10-K/A`)
- Contract and MSA revisions, credit agreement exhibits (`Ex-10.x`), and term sheets
- Vendor pricing pages, Terms of Service, privacy policies, and regulatory guidance updates
- Policy manuals, exam responses, and technical specifications

Existing options fail in opposite ways:

1. **Raw `diff` / byte diff** drowns in HTML tags, inline CSS, pagination headers, and reflowed whitespace.
2. **HTML-to-text conversion followed by diff** destroys byte offsets into the original file, breaking cryptographic provenance and making it impossible to prove where a change occurred in the `--lock`-pinned artifact.
3. **LLM-only comparison** burns tokens on unchanged text, misses quiet three-word edits buried in long exhibits, and cannot prove exhaustiveness.

`redline` solves this as a **sensor-agnostic, deterministic primitive**: normalize away layout noise while keeping an exact byte-offset map back into the original files, diff deterministically, classify hunk types cheaply, and emit a sealed, redacted-by-default receipt (`redline.report.v1`) that pairs directly with `lock`, `verify`, and `pack`.

---

## 2. Core Pipeline: Canonicalize-with-Source-Map

Every run executes four pure, deterministic stages:

### Stage 1 — Input Pinning & Format Detection
- Accepts two input documents (`old`, `new`), either as paths (CLI) or `&[u8]` byte slices (pure-Rust library API).
- Optionally verifies both inputs against a `lock.v0` lockfile (`--lock`, supporting `blake3:` and `sha256:` digests).
- Selects canonicalization mode (`--mode auto|raw|whitespace|markdown|html-text|xml-text`, with optional `docx` via `word/document.xml`).

### Stage 2 — Canonicalization with Raw Byte Source Map
Rather than discarding source offsets during cleanup, canonicalization produces:

```text
CanonicalDoc {
    text: String,                      // normalized UTF-8 text used for diffing
    spans: Vec<(usize, Range<usize>)>, // canonical byte offset -> exact raw [start, end) byte span in original input
    sections: Vec<SectionAnchor>,      // heading / Item / anchor boundaries with both canonical & raw spans
}
```

Mode rules:
- `raw`: 1:1 byte identity (UTF-8 validated).
- `whitespace`: normalizes CRLF/CR -> LF, collapses runs of Unicode whitespace, strips trailing spaces while mapping each canonical token back to its exact raw byte range.
- `markdown`: uses offset-preserving parsing (e.g., `pulldown-cmark::Parser::into_offset_iter`) to preserve heading hierarchy and exact source `Range<usize>` byte spans.
- `html-text` / `xml-text`: uses a streaming, allocation-light tokenizer (not a DOM tree that drops byte positions) that:
  - skips `<script>`, `<style>`, `<head>`, comments, and tag markup,
  - emits structural paragraph/block breaks on block elements (`<p>`, `<div>`, `<tr>`, `<li>`, `<h1>`–`<h6>`, `<br>`),
  - decodes HTML/XML entities (`&nbsp;`, `&amp;`, `&#8217;`, etc.) while mapping each decoded character back to the entity's exact raw byte span,
  - optionally strips boilerplate page numbers / running headers via deterministic rules.

### Stage 3 — Optional Region / Span Targeting
Callers can diff whole documents or narrow scope before diffing:
- `--region <regex>` / `--section <label>`: align and diff only matching sections (e.g., `Item 5.02`, `Item 1A`, `Section 7\.1`).
- `--old-span <start:end>` / `--new-span <start:end>`: diff two explicit raw byte ranges directly.

If a requested region exists in only one document, report it cleanly as `section_added` or `section_removed` (never silently drop it).

### Stage 4 — Deterministic Diff & Hunk Classification
- Align sections (when headings exist), then run a deterministic diff algorithm (`imara-diff` Histogram/Myers or `similar` Patience/Myers) at paragraph -> sentence -> word/token granularity (`unicode-segmentation`).
- Project every diff hunk back through the source map to emit **exact raw `[start_byte, end_byte]` spans** in both `old` and `new`.
- Deterministically classify every hunk so downstream agents can filter noise at zero token cost:
  - `formatting_only`: raw bytes differ, canonical text is identical.
  - `numeric_changed`: hunk alters one or more numbers, currency values, percentages, or dates.
  - `clause_added` / `clause_removed`: whole sentence, bullet, paragraph, or section inserted or deleted.
  - `text_modified`: inline prose edit within an existing sentence/paragraph.
- Compute summary metrics: changed hunk counts by class, section coverage, token churn ratio, and whether all raw differences were formatting-only.

---

## 3. Output & Spine Contract

`redline` follows all CMD+RVL spine conventions:

### Exit Codes
| Code | Meaning |
| :--- | :--- |
| `0` | `NO_SEMANTIC_CHANGE` — identical bytes or `formatting_only` differences under the selected mode |
| `1` | `CHANGES_FOUND` — one or more semantic hunks detected |
| `2` | `REFUSAL` — typed `E_UPPERCASE` refusal with structured `detail` and mechanical `next_command` |

### Zero-Retention Default (`--explicit` Opt-In)
- **Default mode (redacted):** Emits exact `old_span` / `new_span` raw byte ranges, section path/label (or hashed label when configured), hunk class, token/char counts, and `blake3:` hashes of the old/new hunk text — **no raw document prose** enters stdout or logs.
- **`--explicit` mode:** Includes bounded raw/canonical excerpts for each hunk so an authorized interpreter (human or agent) can read only the changed spans without loading the full document.

### Structured Report (`redline.report.v1`)
```json
{
  "schema_version": "redline.report.v1",
  "outcome": "CHANGES_FOUND",
  "mode": "html-text",
  "inputs": {
    "old": { "path": "...", "bytes": 184220, "digest": "blake3:..." },
    "new": { "path": "...", "bytes": 185914, "digest": "blake3:..." }
  },
  "summary": {
    "total_hunks": 3,
    "semantic_hunks": 2,
    "formatting_only_hunks": 1,
    "numeric_changed_hunks": 1,
    "churn_ratio": 0.0042
  },
  "hunks": [
    {
      "id": "hunk-001",
      "class": "numeric_changed",
      "section": "Item 5.02",
      "old_span": [41208, 41251],
      "new_span": [41390, 41436],
      "old_digest": "blake3:...",
      "new_digest": "blake3:...",
      "old_tokens": 7,
      "new_tokens": 7
    }
  ]
}
```

### CLI & Agent Ergonomics
- Primary invocation: `redline <old> <new> [--mode ...] [--region ...] [--lock ...] [--json] [--explicit]`
- Plugin compatibility: invocable as `cmdrvl redline ...` via git-style `cmdrvl-<subcommand>` dispatch.
- Self-description: `--describe`, `--schema`, `capabilities --json`, `robot-docs guide`, `--robot-triage`.
- Ambient provenance: writes hash-chained witness records unless `--no-witness` is passed.
- Typed refusals (`exit 2`), e.g.:
  - `E_BINARY_INPUT` (non-UTF-8 / PDF / image binary passed directly)
  - `E_LOCK_MISMATCH` (file digest does not match `--lock`)
  - `E_REGION_INVALID` (invalid regex or out-of-bounds explicit byte span)
  - `E_EMPTY_INPUT` / `E_IO`

---

## 4. Architecture & Crate Constraints

- **Pure-Rust core + thin CLI split:**
  - `redline-core` (or `lib.rs` with optional `cli` feature): `#![forbid(unsafe_code)]`, zero filesystem/OS dependencies, takes `&[u8]` slices, compiles cleanly to `wasm32-unknown-unknown` so browser tools (e.g., client-side Pack Explorer) can render exact redlines locally.
  - Thin CLI wrapper (`src/main.rs`): handles paths, `lock.v0`, witness logging, and human/JSON formatting.
- **Zero C FFI, zero DuckDB, zero network/LLM calls.**
- **Candidate pure-Rust dependencies:**
  - `imara-diff` or `similar` (diff engine)
  - `pulldown-cmark` (Markdown with `into_offset_iter()`)
  - Streaming byte-offset HTML/XML tokenizer (custom ~250 LOC or `quick-xml`)
  - `unicode-segmentation`, `unicode-normalization`, `regex`
  - `blake3`, `sha2`, `serde`, `serde_json`
  - Optional `zip` + `quick-xml` feature for `.docx` (`word/document.xml`)

---

## 5. Non-Goals (v0.1)

- **No raw PDF parsing:** PDFs are compressed glyph/drawing streams, not UTF-8 text. Extract text upstream (e.g., via OCR or PDF text extraction), pin the extracted text with `lock`, and diff that artifact with `redline`.
- **No tabular cell math:** Use [`shape`](https://github.com/cmdrvl/shape) + [`rvl`](https://github.com/cmdrvl/rvl) for tables, CSVs, Parquet, and Excel spreadsheets.
- **No LLM inference or semantic scoring inside `redline`:** `redline` stays 100% deterministic. An interpreter reads the bounded hunks, reasons over economic or legal meaning, and grounds claims back to exact spans via [`verify`](https://github.com/cmdrvl/verify).
- **No sensor-specific fetching:** Fetching SEC filings, scraping web pages, or polling APIs belongs in upstream sensors/MCP servers, not in `redline`.

---

## 6. Verification & Invariants (When Implementing)

Before `v0.1.0` ships, the test suite must prove:
1. **Byte-span exactness invariant (property test):** For every emitted hunk, slicing `old_bytes[old_span.0..old_span.1]` and `new_bytes[new_span.0..new_span.1]` is valid UTF-8 (for text formats) and re-canonicalizes to the exact diffed hunk text.
2. **Idempotence & symmetry:** Diffing `A` vs `A` always returns exit `0` with zero semantic hunks; swapping `(A, B)` -> `(B, A)` swaps `clause_added` <-> `clause_removed` and preserves span exactness.
3. **Formatting invariance:** Re-wrapping whitespace, changing CSS attributes, or inserting HTML comments changes raw bytes but yields `NO_SEMANTIC_CHANGE` (exit `0`, `formatting_only` hunks only) in `html-text` / `whitespace` modes.
4. **Zero-retention test:** Default output without `--explicit` never leaks raw input tokens (`spine-rules` redaction check).
5. **`wasm32-unknown-unknown` build gate:** `cargo check --target wasm32-unknown-unknown --no-default-features` passes in CI.
