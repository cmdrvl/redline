# redline robot-docs guide

redline diffs two versions of a document and reports every change as exact byte spans in the original files. It is deterministic: same bytes and flags in, same bytes out. It makes no network or model calls.

redline's agent discovery surface is read-only. The discovery commands never read documents, write witness ledgers or touch the network.

Commands:
- redline <old> <new> --json
- redline capabilities --json
- redline robot-docs guide
- redline doctor [--json]
- redline doctor capabilities --json
- redline doctor triage --json
- redline doctor robot-docs
- redline doctor --fix
- redline witness <query|last|count> [--json]
- redline --describe | --schema | --version

Aliases (same cmdrvl.doctor.v1 output):
- redline --robot-triage  ->  redline doctor triage --json
- redline doctor --robot-triage  ->  redline doctor triage --json
- redline doctor health [--json]  ->  redline doctor [--json]

Exit codes:
- 0: NO_SEMANTIC_CHANGE (doctor: healthy)
- 1: CHANGES_FOUND, a normal expected outcome (doctor: unhealthy, an error finding)
- 2: refusal or command-line usage error

set -e safety:
- Pass `--exit-zero-on-change` to exit 0 on CHANGES_FOUND, or guard the call: `redline a.htm b.htm || [ $? -eq 1 ]`.
- Refusals still exit 2.

Modes (--mode):
- auto (default): html-text when both inputs open as HTML documents (`<!DOCTYPE html` or `<html>`, EDGAR wrapper lines allowed in front), whitespace when both open with text. Refuses with E_MODE_AMBIGUOUS when they differ or when one opens with some other tag (an HTML fragment, Markdown, tagged text): pass --mode html-text to ignore tags or --mode whitespace to treat them as text.
- raw: every byte counts.
- whitespace: runs of whitespace collapse to one space; re-wrapping is not a change.
- html-text: markup, comments, scripts, styles, the document title and the hidden Inline XBRL header are dropped; entities are decoded; whitespace collapses.

Reading the report (redline.report.v1):
- `outcome`: NO_SEMANTIC_CHANGE, CHANGES_FOUND or REFUSAL.
- `hunks[].old_span` and `new_span`: half-open byte ranges [start, end) into the ORIGINAL files, not the normalized text. An empty span marks a position: text was inserted or removed there.
- `hunks[].class`: formatting_only (raw bytes differ, text identical), clause_added, clause_removed (whole sentences or blocks), numeric_changed (an edit that alters a number or the sign, unit, month or magnitude word attached to one), text_modified (any other edit).
- `hunks[].numeric`: true when the hunk adds, removes or alters a number, whatever its class.
- `summary.churn_ratio`: share of all tokens that sit inside semantic hunks.
- To read a change, slice the original file at the span. Skip formatting_only hunks unless markup matters to you.

Redaction (--explicit):
- Default output is redacted for zero-retention safety: spans, classes, counts and digests only, no document text.
- Pass `--explicit` to add `old_text` and `new_text` (normalized text, cut at 2000 bytes) to each semantic hunk.
- Use `--explicit` for local non-sensitive runs; omit it for logged or shared output.
- `old_digest` and `new_digest` are BLAKE3 keyed hashes of the span's raw bytes. The key is derived from the whole input file, so a holder of the file can verify a span and nobody else can test guesses against the digest.

Refusals:
- A refusal is JSON on stdout with `--json` (stderr text without), exit 2, with `refusal.code`, `refusal.detail` and, when one exists, `refusal.next_command`.
- E_IO, E_EMPTY_INPUT, E_BINARY_INPUT, E_MODE_UNSUPPORTED, E_MODE_AMBIGUOUS.
- PDFs and other binaries are refused. Extract text first, then diff the extracted text.

Composition:
- Pin both inputs with `lock` before diffing so spans refer to known bytes.
- Hand the hunks to an interpreter, and ground its claims back to the spans with `verify`.
- Use `shape` and `rvl` for tables, CSVs and spreadsheets. redline is for prose.

Repair policy: redline doctor --fix is unavailable and exits 2 without stdout. Use redline doctor triage --json or redline capabilities --json for read-only diagnostics.
