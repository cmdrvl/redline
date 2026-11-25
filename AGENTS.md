# AGENTS.md - redline

> Repo-specific guidelines. Inherits shared rules from [`../AGENTS.md`](../AGENTS.md).

---

## Quick Reference

```bash
# Find and claim work
br ready --limit 0 --json
br show <id> --json
br update <id> --status in_progress --json

# Verify changes
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown --no-default-features

# Agent discovery surfaces
redline capabilities --json
redline robot-docs guide
redline doctor triage --json
redline doctor --fix  # exits 2 with read-only alternatives
```

Do not bump `Cargo.toml` for normal work. A push to `main` that changes `Cargo.toml` triggers the release and Homebrew tap workflow; releases are cut only when the operator asks. Keep `operator.json` `tool_version` equal to the Cargo version (a test enforces it).

---

## redline - What This Project Does

`redline` diffs two versions of a prose document (plain text or HTML today) and reports every change as exact byte spans in the original files. It is the prose twin of `rvl`, which does the same job for tables.

Position in the stack:

```text
lock (pin old/new bytes) -> redline (what changed, as raw byte spans) -> interpreter reads only the hunks
                                        |
                                        v
                         verify (ground claims to spans) / pack (seal)
```

### What redline Owns

- Canonicalization of each input into normalized text plus a source map from canonical offsets back to raw byte spans.
- Deterministic diff over the canonical text.
- Projection of every hunk back to raw byte spans in both `old` and `new`.
- Deterministic hunk classification: `formatting_only`, `numeric_changed`, `clause_added`, `clause_removed`, `text_modified`.
- The `redline.report.v1` report, redacted by default.
- Typed refusals with a mechanical next step.
- Witness records for its own runs, and the read-only agent discovery surfaces.

### What redline Does Not Own

- PDF parsing or OCR. Extract text upstream, pin it with `lock`, diff the extracted text.
- Tables, CSVs, Parquet, spreadsheets. Those belong to `shape` and `rvl`.
- Meaning. No LLM calls, no semantic scoring, no judgment about whether a change matters.
- Fetching. No network access, no SEC or web retrieval.

### Source of Truth

- **Spec:** [`docs/SPEC.md`](./docs/SPEC.md). Sections 1 to 6 are the design. Section 7 lists what is implemented, what is not, and the decisions that settle points the design left open.
- **Report schema:** [`docs/redline.report.v1.schema.json`](./docs/redline.report.v1.schema.json), compiled into `--schema`. Tests validate real output against it.
- **Machine contract:** [`operator.json`](./operator.json), compiled into `--describe`.
- **User docs:** [`README.md`](./README.md).

Do not invent behavior that is not in the spec. Where the spec is silent or contradicts itself, file a bead and get a ruling, then change the spec (section 7) and the code in the same unit of work. Do not edit the spec to match an implementation shortcut.

---

## Core Behavior

- Every run emits exactly one outcome: `NO_SEMANTIC_CHANGE` (exit 0), `CHANGES_FOUND` (exit 1) or `REFUSAL` (exit 2).
- Same input bytes and flags produce byte-identical output. No clocks, randomness, hash-map iteration order or locale in the report.
- Every emitted span is a half-open range into the original input bytes, never the canonical text.
- `redline B A` is the exact mirror of `redline A B`.
- Refuse with a typed `E_UPPERCASE` code and a concrete next command instead of guessing.
- Default output carries no document text, on stdout, on stderr or in the witness ledger. Text appears only under `--explicit`, and never in the ledger.
- Output wording and field order are part of the public contract. No banners, progress text or ad-hoc fields.

### Output routing

- **Human (default):** verdicts go to stdout; `REFUSAL` goes to stderr.
- **`--json`:** exactly one JSON object on one line on stdout for all outcomes; stderr is for process-level failures only.
- Discovery and doctor surfaces are read-only. They must not read documents, write the ledger or touch the network. `doctor` exits 0 or 1, never 2.

---

## Project Structure

| Path | Purpose |
|------|---------|
| `src/lib.rs` | Library root and re-exports |
| `src/canon/mod.rs` | `CanonicalDoc`, the source map, the `Builder` every mode writes through |
| `src/canon/text.rs` | Plain-text scanning: whitespace runs, block breaks |
| `src/canon/html.rs` | HTML scanner: tags, comments, dropped elements, head handling |
| `src/canon/entities.rs` | Character reference decoding |
| `src/tokens.rs` | Tokens, blocks, sentence starts |
| `src/diff.rs` | Two-level diff, slider heuristic, change merging |
| `src/classify.rs` | Hunk classes |
| `src/engine.rs` | `compare`: validation, mode choice, hunks, digests, summary |
| `src/report.rs` | `redline.report.v1` types |
| `src/refusal.rs` | Refusal codes. The only file that may hold `"E_..."` code literals |
| `src/cli/` | CLI behind the `cli` feature: args, human output, witness, doctor |
| `src/main.rs` | Entry point |
| `tests/invariants.rs` | Property tests for the SPEC section 6 invariants |
| `tests/regressions.rs` | Defects found in review, one test each: missed changes in broken HTML, cost bounds, classification |
| `tests/cli.rs` | Binary tests, each in a scratch `HOME` and ledger |
| `tests/golden_rules.rs` | The shared spine-rules suite |
| `operator.json` | Machine-readable operator contract |

---

## Architecture Constraints

- The core library takes `&[u8]`, has no filesystem, clock or OS dependencies, and builds for `wasm32-unknown-unknown` with `--no-default-features`. Anything that touches files, env vars or time belongs in `src/cli/`.
- `#![forbid(unsafe_code)]`. No C FFI (`blake3` is built with its `pure` feature), no DuckDB, no network, no LLM.
- Every mode writes canonical text through `canon::Builder`, which is what keeps the source map exact. Do not build canonical text any other way.
- The HTML path is a scanner that keeps byte positions. Do not introduce a DOM parser that drops them.
- No `HashMap` on an output path. Report types are structs and vectors; the witness record sorts its keys by hand.

---

## Toolchain

- **Language:** Rust, stable
- **Package manager:** Cargo only
- **Edition:** Rust 2024
- **Unsafe code:** forbidden
- **Dependency policy:** pure-Rust, explicit and small. Licenses must be compatible with MIT.

| Crate | Purpose |
|-------|---------|
| `imara-diff` | Histogram diff and the slider postprocessor |
| `unicode-segmentation` | Word and sentence boundaries |
| `entities` | The WHATWG named character reference table |
| `blake3` | Input digests, keyed hunk digests, witness ids |
| `serde` / `serde_json` | Report and witness serialization |
| `clap` | CLI parsing (`cli` feature only) |
| `proptest`, `jsonschema`, `spine-rules`, `tempfile` | Tests only |

Release profile, matching the other spine tools:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

---

## Tests

The spec's section 6 invariants are the acceptance bar, and each has a real test in `tests/invariants.rs`. When you add a mode or change canonicalization, extend the generators there so the new path is covered by the same properties.

| Change area | Useful command |
|-------------|----------------|
| Canonicalization, diff, classification | `cargo test --lib` and `cargo test --test invariants` |
| CLI flags, exits, output routing, witness, redaction | `cargo test --test cli` |
| Operator contract, doctor shape, refusal codes | `cargo test --test golden_rules` |

Rules:

- Tests must not touch the real `~/.cmdrvl`. Use the `Sandbox` helper in `tests/cli.rs`, and always pass `--no-witness` when running the binary by hand. `tests/golden_rules.rs` leaves `fixture_success_args` unset for this reason; do not set it.
- A property that has never failed proves little. When you add one, break the code on purpose once and confirm the property catches it.
- Goldens and expected values change only when the spec changes, never to turn a failing run green.
- Check behavior on real documents before claiming a quality improvement. Synthetic fixtures do not show alignment problems.

Before committing changed code, run UBS on the changed files. It reports the entity decoder in `src/canon/entities.rs` as a JWT decode; that finding is a false positive.

---

## CI and Release

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `.github/workflows/ci.yml` | PR, push to `main`, published release | fmt, clippy, test, wasm32 library check |
| `.github/workflows/release.yml` | Manual dispatch or push to `main` affecting `Cargo.toml` | Builds archives for five targets, signs checksums, publishes the release, updates the Homebrew tap |

Release constraints:

- Do not change the `Cargo.toml` version unless the operator asks. Bump `operator.json` `tool_version` in the same commit.
- `Cargo.lock` is committed; release builds use `--locked`.
- The Homebrew job needs the repo secret `HOMEBREW_TAP_TOKEN`.

---

## Beads Workflow

Beads is the source of truth for task state. It never runs git commands.

```bash
br ready --limit 0 --json
br show <id> --json
br update <id> --status in_progress --json
br comments add <id> "status or evidence note"
br close <id> --reason "Completed: <tests/evidence>"
br sync --flush-only
```

Rules:

- Run `br` from inside this repo. Do not point `--db` at it from another directory.
- Use JSON output for agent-readable commands. `--limit 0` avoids silent truncation at 20 items.
- Claim a bead with `--status in_progress` before editing.
- Close with cited evidence: test names and commit SHA.
- After `br sync --flush-only`, stage `.beads/` yourself.

Use `bv` only with robot flags (`bv --robot-next`, `bv --robot-triage`, `bv --robot-plan`). Bare `bv` opens a TUI.

---

## Agent Coordination

For multi-agent sessions in this repo:

- Register in Agent Mail for this repository.
- Reserve only the exact files you will edit.
- Use the bead ID as the Mail `thread_id`, subject prefix and reservation reason.
- Release reservations when done.

Never stash, revert, delete or overwrite another agent's work. Treat unexpected changes as concurrent work and leave unrelated dirty files alone.

---

## Session Completion

1. Confirm the work matches `docs/SPEC.md`, and that section 7 is current.
2. Run the quality gate (fmt, clippy, test, wasm check).
3. Run `ubs` on changed files.
4. Close finished beads with evidence, or leave them open with a comment saying what remains.
5. `br sync --flush-only`.
6. Commit and push:
   ```bash
   git add .beads/ <changed-files>
   git commit -m "<type>: <summary> (<bead-id>)"
   git pull --rebase
   git push origin main
   ```
7. Confirm `git status` is up to date with `origin/main`.

This repo has `main` only. There is no `master` mirror.
