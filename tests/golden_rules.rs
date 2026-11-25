//! Family conventions, enforced by the shared spine-rules suite.
//!
//! `fixture_success_args` is deliberately not set. The suite's witness checks
//! for a successful run establish their baseline against the default ledger,
//! which would append a record to the developer's real `~/.cmdrvl` on every
//! test run. Those rules (R-001 success, R-008, R-017) are covered in
//! `tests/cli.rs` with the ledger pointed at a scratch directory.

use spine_rules::golden_rules_suite;

golden_rules_suite! {
    binary: env!("CARGO_BIN_EXE_redline"),
    operator_json: include_str!("../operator.json"),
    source_files: &["src/refusal.rs"],
    fixture_refusal_args: &[
        "/nonexistent/old.htm",
        "/nonexistent/new.htm",
        "--json",
        "--no-witness",
    ],
    // A `since` far in the future can match no ledger record.
    fixture_empty_query_args: &["witness", "query", "--since", "2999-01-01T00:00:00Z", "--json"],
}
