# redline

> **Status: Planning / Pre-implementation.** This repository currently holds the design specification ([`docs/SPEC.md`](./docs/SPEC.md)). Implementation will begin after core spine consolidation.

**Deterministic prose and document byte-span diff for the [CMD+RVL](https://github.com/cmdrvl) spine.**

[`rvl`](https://github.com/cmdrvl/rvl) reveals what changed between two **tables**. `redline` is its unstructured-text twin: a pure-Rust, `wasm32`-clean library and CLI that diffs two text, Markdown, HTML/iXBRL, or XML documents, suppresses layout noise, and projects every semantic change back to **exact `[start_byte, end_byte]` spans** in the original `--lock`-pinned bytes.

## Where `redline` sits in the spine

| | Tabular | Prose / Documents |
| :--- | :--- | :--- |
| **Diff / What changed** | [`rvl`](https://github.com/cmdrvl/rvl) | **`redline`** |
| **Ground / Verify claims** | [`verify`](https://github.com/cmdrvl/verify) | [`verify`](https://github.com/cmdrvl/verify) (quote, span & numeric grounding) |
| **Pin & Seal** | [`lock`](https://github.com/cmdrvl/lock) + [`pack`](https://github.com/cmdrvl/pack) | [`lock`](https://github.com/cmdrvl/lock) + [`pack`](https://github.com/cmdrvl/pack) |

## Planned capabilities

- **Canonicalize with a raw byte source map:** Strip HTML tags, CSS, page numbers, and formatting noise while maintaining an exact `canonical_offset -> raw_byte_span` map back into the original locked files.
- **Region & section targeting:** Diff an entire document or target a specific section/heading anchor (e.g., `Item 5.02`, `Section 7.1`) or explicit byte range.
- **Zero-token noise filtering:** Deterministically classify hunks (`formatting_only`, `numeric_changed`, `clause_added`, `clause_removed`, `text_modified`) so downstream agents only inspect real semantic deltas.
- **Zero-retention by default:** Emits exact byte spans, section labels, counts, and hunk hashes without leaking raw document text unless `--explicit` is passed.
- **Pure Rust & browser-ready:** `#![forbid(unsafe_code)]`, zero C FFI, no DuckDB, `wasm32-unknown-unknown`-clean core library plus thin CLI (`cmdrvl redline` / `redline`).

See [`docs/SPEC.md`](./docs/SPEC.md) for the full design specification and CLI contract.

## License

MIT — see [`LICENSE`](./LICENSE).
