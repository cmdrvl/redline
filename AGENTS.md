# AGENTS.md - redline

> Repo-specific guidelines. Inherits shared rules from [`../AGENTS.md`](../AGENTS.md).

---

## Status

Pre-implementation. The repo holds the design spec ([`docs/SPEC.md`](./docs/SPEC.md)) and no Rust code yet. There is no `Cargo.toml`, no CI workflow and no release workflow. Sections below that describe code state the target the spec sets; update them as the code lands.

---

## Quick Reference

```bash
# Find and claim work
br ready --limit 0 --json
br show <id> --json
br update <id> --status in_progress --json

# Verify changes (once Cargo.toml exists)
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown --no-default-features
```

---

## redline - What This Project Does

`redline` diffs two versions of a prose document (plain text, Markdown, HTML/iXBRL, XML) and reports every change as exact byte spans in the original files. It is the prose twin of `rvl`, which does the same job for tables.

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

### What redline Does Not Own

- PDF parsing or OCR. Extract text upstream, pin it with `lock`, diff the extracted text.
- Tables, CSVs, Parquet, spreadsheets. Those belong to `shape` and `rvl`.
- Meaning. No LLM calls, no semantic scoring, no judgment about whether a change matters.
- Fetching. No network access, no SEC or web retrieval.

### Source of Truth

- **Spec:** [`docs/SPEC.md`](./docs/SPEC.md) - pipeline, modes, report schema, exit codes, refusal codes, invariants.
- **User docs:** [`README.md`](./README.md).

Do not invent behavior that is not in the spec. Where the spec is silent or contradicts itself, file a bead and get a ruling, then change the spec and the code in the same unit of work. Do not edit the spec to match an implementation shortcut.

---

## Core Behavior

- Every run emits exactly one outcome: `NO_SEMANTIC_CHANGE` (exit 0), `CHANGES_FOUND` (exit 1) or `REFUSAL` (exit 2).
- Same input bytes and flags produce byte-identical output. No clocks, randomness, hash-map iteration order or locale in the report.
- Every emitted span indexes the original input bytes, never the canonical text.
- Refuse with a typed `E_UPPERCASE` code and a concrete next command instead of guessing.
- Default output carries no document prose. Raw or canonical excerpts appear only under `--explicit`.
- Output formatting is part of the public contract. No banners, progress text or ad-hoc fields.

---

## Architecture Constraints

- Core library takes `&[u8]`, has no filesystem or OS dependencies, and builds for `wasm32-unknown-unknown` with `--no-default-features`.
- The CLI is a thin wrapper over the core: paths, `lock.v0` checks, witness records, human and JSON formatting.
- `#![forbid(unsafe_code)]`. No C FFI, no DuckDB, no network, no LLM.
- The HTML/XML path uses a streaming tokenizer that keeps byte positions. Do not introduce a DOM parser that drops them.

---

## Toolchain

- **Language:** Rust, stable
- **Package manager:** Cargo only
- **Edition:** Rust 2024
- **Unsafe code:** forbidden
- **Dependency policy:** pure-Rust, explicit and small. Candidates are listed in `docs/SPEC.md` section 4.

Release profile, matching the other spine tools:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Do not bump the `Cargo.toml` version or cut a release unless the operator asks.

---

## Tests

The spec's section 6 invariants are the acceptance bar for `v0.1.0`. Each needs a real test, not a fixture presented as proof:

1. Byte-span exactness (property test).
2. Idempotence (`A` vs `A`) and symmetry (`A,B` vs `B,A`).
3. Formatting invariance in `html-text` and `whitespace` modes.
4. Zero retention in default output.
5. `wasm32-unknown-unknown` build of the core.

Goldens are regenerated only when the spec changes, never to turn a failing run green.

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

1. Confirm the work matches `docs/SPEC.md`.
2. Run the quality gate (fmt, clippy, test, wasm check) once code exists.
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
