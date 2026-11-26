# redline

> *Two versions of a document. Three words changed on page 84. Here are the bytes.*

<div align="center">

[![CI](https://github.com/cmdrvl/redline/actions/workflows/ci.yml/badge.svg)](https://github.com/cmdrvl/redline/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub release](https://img.shields.io/github/v/release/cmdrvl/redline)](https://github.com/cmdrvl/redline/releases)

**Two filings. Five megabytes each. What actually changed?**

```bash
brew install cmdrvl/tap/redline
```

</div>

---

An 8-K and its amendment. Last year's 10-K and this year's. A contract and its redraft. You need to know what changed, and you need to point at it.

A byte diff drowns in tags, inline CSS and reflowed whitespace. Converting to text first loses the offsets, so you can no longer say where in the filed document a change sits. Asking a model to compare them spends tokens on the 99% that did not change and cannot show it read everything.

**redline strips the layout noise, diffs the text, and reports every change as an exact byte range in the original files.** It classifies each change so the noise can be filtered without reading it. It is deterministic, runs offline, and by default prints no document text at all.

[`rvl`](https://github.com/cmdrvl/rvl) does this for tables. `redline` is its twin for prose.

### What makes this different

- **Spans point at the original bytes**: every hunk is `[start, end)` in the files you gave it, not in some converted copy. Slice the file and you have the change.
- **Formatting is separated from substance**: restyled tags, re-wrapped lines and `&nbsp;` versus a space are reported as `formatting_only` and do not count as changes.
- **Changes are classified for free**: `numeric_changed`, `clause_added`, `clause_removed`, `text_modified`. An agent can skip straight to the hunks that touch numbers.
- **Redacted by default**: spans, classes, counts and keyed digests only. The report can be logged or shared without carrying the document.
- **Refuses instead of guessing**: a PDF, an empty file or a mismatched pair gets a typed refusal and a next step, never a made-up verdict.
- **Fast**: two 5.5 MB 10-K filings (about 100,000 words each) in about half a second.

---

## Quick Example

> **Redacted by default.** redline is zero-retention safe out of the box: without `--explicit` it prints where the changes are and what kind they are, and none of the text. Add `--explicit` to see the changed text.

```bash
$ redline 8k_old.htm 8k_new.htm --explicit
```

```text
REDLINE

CHANGES FOUND

Compared: 8k_old.htm -> 8k_new.htm
Mode: html-text
Hunks: 2 semantic (1 clause_added, 0 clause_removed, 1 numeric_changed, 0 text_modified), 2 formatting_only
Tokens: old=129 new=145 churn=6.57%

hunk-001  formatting_only  old[0,703)  new[0,722)  tokens 61 -> 61
hunk-002  numeric_changed  old[781,788)  new[800,807)  tokens 1 -> 1  numeric
  - 450,000
  + 475,000
hunk-003  clause_added     old[843,843)  new[862,928)  tokens 0 -> 16  numeric
  +  Ms. Avery will also receive a one-time signing bonus of $100,000.
hunk-004  formatting_only  old[843,1178)  new[928,1281)  tokens 0 -> 0
```

The amendment changed the font, renamed `<b>` to `<strong>`, added a CSS class to every paragraph and re-wrapped a line. None of that is a change. The salary and the new sentence are.

The spans are real offsets. `head -c 807 8k_new.htm | tail -c 7` prints `475,000`.

Both fixtures are in [`tests/fixtures`](./tests/fixtures).

## The Three Outcomes

Every run ends in exactly one outcome.

| Outcome | Exit | Meaning |
| :--- | :--- | :--- |
| `NO_SEMANTIC_CHANGE` | `0` | The bytes are identical, or every difference is `formatting_only` under the chosen mode. |
| `CHANGES_FOUND` | `1` | At least one semantic hunk. This is a normal result, not an error. |
| `REFUSAL` | `2` | No verdict. The output names a typed `E_` code, what was seen, and what to run next. |

Exit `1` is expected for a diff tool. In a `set -e` script pass `--exit-zero-on-change`, or guard the call: `redline a.htm b.htm || [ $? -eq 1 ]`. Refusals still exit `2`.

## How It Works

1. **Check the inputs.** Both must be non-empty UTF-8 text. Anything else is refused.
2. **Normalize with a source map.** Each input becomes normalized text, and every character of that text remembers the raw byte range it came from: a decoded `&amp;` maps to all five bytes of the entity, a collapsed run of whitespace maps to the whole run.
3. **Diff in two levels.** Whole paragraphs are matched first, which anchors the alignment on text that certainly did not change. Words are then diffed only between the anchors (histogram diff, via [`imara-diff`](https://crates.io/crates/imara-diff)).
4. **Clean up.** An added or removed sentence is slid onto sentence boundaries. Edits separated only by punctuation are joined, so `March 1, 2024` to `April 15, 2025` is one change. When a paragraph is rewritten, stray matching words are folded into one change in place of dozens of fragments.
5. **Project and classify.** Each change is mapped back to raw byte spans in both files and given a class. Stretches the diff found equal are compared byte for byte; where they differ, that is a `formatting_only` hunk.

Swapping the two inputs gives the mirror-image report: same spans, `clause_added` and `clause_removed` exchanged.

### Modes

| `--mode` | What is ignored | Use for |
| :--- | :--- | :--- |
| `auto` (default) | Picks `html-text` when both inputs open as HTML documents (`<!DOCTYPE html` or `<html>`; EDGAR wrapper lines may come first) and `whitespace` when both open with text. Refuses when they differ, or when one opens with some other tag and could be an HTML fragment, Markdown or tagged text. | Most runs |
| `raw` | Nothing. Every byte counts. | Config files, anything where whitespace matters |
| `whitespace` | Runs of whitespace collapse to one space. Re-wrapping is not a change. A blank line is a paragraph break. | Plain text, extracted PDF text |
| `html-text` | Tags, attributes, comments, `<script>`, `<style>`, the document title and the hidden Inline XBRL header (`<ix:header>`). Entities are decoded. Whitespace collapses. | SEC filings, web pages, HTML contracts |

### Hunk classes

| Class | Meaning |
| :--- | :--- |
| `formatting_only` | Raw bytes differ, normalized text is identical. Not a semantic change. |
| `clause_added` / `clause_removed` | One or more whole sentences or paragraphs exist on only one side. |
| `numeric_changed` | An edit inside a sentence that changes a number, or the sign, unit, month or magnitude word attached to one (`$5 million` to `$5 billion`). |
| `text_modified` | Any other edit inside a sentence. |

Every hunk also carries `numeric: true` when it adds, removes or alters a number, whatever its class. An added sentence that contains a dollar amount is `clause_added` with `numeric: true`.

## How redline Compares

| | `diff` / `git diff` | HTML-to-text, then diff | Ask an LLM | `redline` |
| :--- | :--- | :--- | :--- | :--- |
| Ignores markup and re-wrapping | No | Yes | Usually | Yes |
| Byte offsets into the original file | Yes, but of noise | No | No | Yes |
| Exhaustive | Yes | Yes | Cannot show it | Yes |
| Deterministic | Yes | Depends on the converter | No | Yes |
| Classifies changes | No | No | Yes, at token cost | Yes, at no cost |
| Safe to log | No | No | No | Yes, by default |

redline does not decide whether a change matters. It finds every change and says where it is. Judgment belongs to whoever reads the hunks.

## Installation

### Homebrew (Recommended)

```bash
brew install cmdrvl/tap/redline
```

### Shell Script

```bash
curl -fsSL https://raw.githubusercontent.com/cmdrvl/redline/main/scripts/install.sh | bash
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/cmdrvl/redline/main/scripts/install.ps1 | iex
```

### From Source

```bash
git clone https://github.com/cmdrvl/redline
cd redline
cargo build --release
./target/release/redline --version
```

Prebuilt binaries are available for x86_64 and ARM64 on Linux and macOS, and x86_64 on Windows. Each release includes SHA256 checksums, cosign signatures, and an SBOM.

## CLI Reference

```text
redline <OLD> <NEW> [OPTIONS]
redline capabilities --json
redline robot-docs guide
redline witness <query|last|count> [OPTIONS]
redline doctor [health|capabilities|triage|robot-docs] [--json]
```

| Option | Meaning |
| :--- | :--- |
| `--mode <MODE>` | `auto` (default), `raw`, `whitespace` or `html-text`. |
| `--json` | Emit one JSON object (`redline.report.v1`) on stdout. |
| `--explicit` | Include the changed text of each semantic hunk. Default output is redacted. |
| `--no-witness` | Do not append a record of this run to the witness ledger. |
| `--exit-zero-on-change` | Exit `0` on `CHANGES_FOUND`. Refusals still exit `2`. |
| `--describe` | Print the operator contract (`operator.v1`) and exit. |
| `--schema` | Print the JSON Schema for `redline.report.v1` and exit. |
| `--version` | Print the version and exit. |

### JSON report

```bash
redline 8k_old.htm 8k_new.htm --json
```

```json
{
  "version": "redline.report.v1",
  "tool": "redline",
  "outcome": "CHANGES_FOUND",
  "mode": "html-text",
  "inputs": {
    "old": { "path": "8k_old.htm", "bytes": 1178, "digest": "blake3:61afd86c..." },
    "new": { "path": "8k_new.htm", "bytes": 1281, "digest": "blake3:1df81143..." }
  },
  "summary": {
    "total_hunks": 4,
    "semantic_hunks": 2,
    "formatting_only_hunks": 2,
    "numeric_changed_hunks": 1,
    "clause_added_hunks": 1,
    "clause_removed_hunks": 0,
    "text_modified_hunks": 0,
    "old_tokens": 129,
    "new_tokens": 145,
    "churn_ratio": 0.065693
  },
  "hunks": [
    {
      "id": "hunk-002",
      "class": "numeric_changed",
      "numeric": true,
      "old_span": [781, 788],
      "new_span": [800, 807],
      "old_digest": "blake3-keyed:9257b927...",
      "new_digest": "blake3-keyed:...",
      "old_tokens": 1,
      "new_tokens": 1
    }
  ],
  "refusal": null
}
```

(Shown indented and abridged. The tool prints one line.)

- **Spans** are half-open byte ranges `[start, end)` into the original file. An empty span such as `[843, 843]` marks a position: text was inserted or removed there.
- **Tokens** are words and punctuation marks. Whitespace is not counted.
- **`churn_ratio`** is the share of all tokens, both files, that sit inside semantic hunks.
- **Hunk digests** are BLAKE3 keyed hashes of the span's raw bytes, with the key derived from the whole input file. Anyone holding the file can recompute a digest and confirm the span. Without the file, a digest cannot be used to test guesses, which a plain hash of a short hunk like `5.25%` would allow.
- With `--explicit`, semantic hunks gain `old_text` and `new_text`: the normalized text, cut at 2000 bytes (`text_truncated: true` when cut).

The full schema is [`docs/redline.report.v1.schema.json`](./docs/redline.report.v1.schema.json), also printed by `redline --schema`.

## Agent / CI Integration

redline follows the conventions shared by the CMD+RVL spine tools.

```bash
redline --describe              # operator.v1 contract: options, exit codes, refusals, side effects
redline capabilities --json     # the same document
redline robot-docs guide        # how to drive the tool and read the report
redline doctor --json           # cmdrvl.doctor.v1 health report
redline doctor triage --json    # health plus what to run next
```

These surfaces are read-only. They never read documents, write the witness ledger or touch the network. `redline doctor --fix` does not exist and says so.

**Witness ledger.** Each comparison appends one record to the ledger shared by the spine tools (`~/.cmdrvl/state/witness/witness.jsonl`, or the path in `EPISTEMIC_WITNESS`): the tool version, the input paths and BLAKE3 digests, the outcome, and a hash of the output. Records chain through `prev`. The ledger never holds document text. Pass `--no-witness` to skip it. A ledger that cannot be written never changes the verdict or the exit code.

```bash
redline witness query --tool redline --outcome CHANGES_FOUND --json
redline witness last
redline witness count
```

## Scripting Examples

Fail a pipeline only when the text changed:

```bash
redline approved.htm candidate.htm --json --exit-zero-on-change \
  | jq -e '.outcome == "NO_SEMANTIC_CHANGE"'
```

List the hunks that touch numbers:

```bash
redline old.htm new.htm --json | jq -c '.hunks[] | select(.numeric)'
```

Print the new text of every semantic hunk straight from the file, without `--explicit`:

```bash
redline old.htm new.htm --json \
  | jq -r '.hunks[] | select(.class != "formatting_only") | "\(.new_span[0]) \(.new_span[1])"' \
  | while read -r start end; do
      tail -c +"$((start + 1))" new.htm | head -c "$((end - start))"; echo
    done
```

Diff the text of two PDFs (redline refuses PDFs; extract first):

```bash
pdftotext -layout old.pdf old.txt && pdftotext -layout new.pdf new.txt
redline old.txt new.txt --mode whitespace
```

## Refusal Codes

| Code | Meaning | Next step |
| :--- | :--- | :--- |
| `E_IO` | An input file could not be read. | Check the path and permissions. |
| `E_EMPTY_INPUT` | An input is zero bytes. | Pass a file that has content. |
| `E_BINARY_INPUT` | An input is not UTF-8 text (PDF, image, UTF-16). | Extract or convert to UTF-8 text, then diff that. |
| `E_MODE_UNSUPPORTED` | The `--mode` value is not available in this release. | Use `auto`, `raw`, `whitespace` or `html-text`. |
| `E_MODE_AMBIGUOUS` | In `auto`, the inputs look like different formats, or one opens with a tag without being an HTML document. | Pass `--mode` explicitly: `html-text` to ignore tags, `whitespace` to treat them as text. |

With `--json` a refusal is the same report object on stdout with `outcome: "REFUSAL"` and a `refusal` block (`code`, `message`, `detail`, `next_command`). Without `--json` it is text on stderr.

## Troubleshooting

- **Everything shows as changed.** The two files are probably different renderings (one HTML, one text) or the document was reordered. Check `--mode`, and see Limitations on moved text.
- **Exit 1 breaks my script.** That is `CHANGES_FOUND`. Use `--exit-zero-on-change`.
- **A curly quote became a straight quote and it counts as a change.** It is one: the characters differ. redline does not fold typography.
- **Hidden text is being diffed.** Only `<script>`, `<style>`, the head's `<title>` and a properly closed `<ix:header>` are skipped. Text hidden with CSS (`display:none`) is still text to redline. When markup is broken, redline keeps text a browser might hide; it would sooner report a change nobody sees than miss one somebody reads.
- **`E_MODE_AMBIGUOUS` on an HTML fragment or a Markdown file.** The file opens with a tag but is not a whole HTML document, so redline cannot tell whether `<...>` is markup or content. Pass `--mode html-text` for a fragment, `--mode whitespace` for Markdown or tagged text.
- **`redline: witness: ...` on stderr.** The ledger could not be written. The result is still correct. Run `redline doctor triage` or pass `--no-witness`.

## Limitations

This is the first release. The design in [`docs/SPEC.md`](./docs/SPEC.md) is larger than what ships today.

- **Not yet built:** Markdown, XML and `.docx` modes; `--region` / `--section` targeting and section labels on hunks; `--lock` verification of inputs against a lockfile. Asking for one of the planned modes is refused with `E_MODE_UNSUPPORTED`.
- **Moved text** is reported as a removal in one place and an addition in another. There is no move detection.
- **Formatting hunks are coarse.** One `formatting_only` hunk covers each unchanged stretch that has any markup difference, from the first differing byte to the last. Use `--mode raw` to see every byte-level difference.
- **HTML is scanned, not parsed into a tree.** Markup boundaries follow the HTML standard's tokenizer rules, but nesting is not modeled, and CSS is not read. Text carried in attributes (`alt`, `value`, `title`) is markup and is not diffed.
- **Huge rewrites are reported coarsely.** When several thousand consecutive words are replaced by several thousand others with no distinctive word in common, that stretch is one hunk. Nothing is dropped; the hunk is just large.
- **`auto` trusts the opening.** A text file whose first characters are `<!DOCTYPE html` or `<html>` is read as HTML. Pass `--mode whitespace` for prose that opens that way, and for EDGAR submissions that mix HTML documents with old tagged-text exhibits.
- **Self-closed `<script/>` is read the XHTML way.** That is right for filings. In plain HTML it means the script body after it is diffed as text.
- **Sentence boundaries** come from the Unicode segmentation rules, which can split after an abbreviation such as `U.S.`. That only affects whether an insertion is labeled `clause_added` or `text_modified`.
- **No Unicode normalization.** `é` as one code point and as `e` plus a combining accent are different text.
- **Inputs are read into memory.** Peak memory was about six times the combined input size on the 10-K pair above (63 MB for 11 MB of HTML).

## FAQ

**Does redline tell me whether a change is material?**
No. It tells you where every change is and what kind of edit it was. Materiality is a judgment for the reader.

**Why are there no page or section numbers?**
Section labels are planned. Today a hunk is located by byte span only.

**Can I diff a PDF?**
Not directly. A PDF is a compressed drawing program, not text. Extract the text, keep the extracted file, and diff that.

**Is the output stable enough to commit?**
Yes. The same input bytes and flags produce the same output bytes. Nothing in the report depends on the clock, the machine or the locale.

**Does it call a model or the network?**
No. Never.

## Spec

The design specification is [`docs/SPEC.md`](./docs/SPEC.md). Its section 7 lists what this release implements and where it departs from the original design.

## Development

Agent guidelines are in [`AGENTS.md`](./AGENTS.md).

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown --no-default-features
```

The core is a library with no filesystem, clock or network access (`src/canon`, `src/tokens.rs`, `src/diff.rs`, `src/classify.rs`, `src/engine.rs`) and builds for `wasm32-unknown-unknown`. The command-line tool is a thin layer over it behind the `cli` feature.

*`redline` is part of the open-source toolchain from the [CMD+RVL](https://cmdrvl.com) lineage and AI enablement practice. MIT-licensed. Contributions welcome from any practice or stack.*
