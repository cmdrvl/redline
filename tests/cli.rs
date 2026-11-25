//! End-to-end tests of the `redline` binary.
//!
//! Every run here gets a scratch `HOME` and a scratch witness ledger, so no
//! test reads or writes the developer's real `~/.cmdrvl`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const OLD: &str = "tests/fixtures/8k_old.htm";
const NEW: &str = "tests/fixtures/8k_new.htm";

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Sandbox {
            dir: TempDir::new().expect("create scratch directory"),
        }
    }

    fn ledger(&self) -> PathBuf {
        self.dir.path().join("witness.jsonl")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_redline"));
        command
            .env("HOME", self.dir.path())
            .env("EPISTEMIC_WITNESS", self.ledger())
            .env_remove("SOURCE_DATE_EPOCH");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run redline")
    }

    fn write(&self, name: &str, contents: &[u8]) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, contents).expect("write scratch file");
        path.to_string_lossy().into_owned()
    }

    fn ledger_records(&self) -> Vec<Value> {
        match std::fs::read_to_string(self.ledger()) {
            Ok(text) => text
                .lines()
                .map(|line| serde_json::from_str(line).expect("ledger line is JSON"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON object")
}

fn assert_matches_schema(report: &Value) {
    let schema: Value =
        serde_json::from_str(include_str!("../docs/redline.report.v1.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(report)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "report violates the schema: {errors:#?}");
}

// ── Verdicts ────────────────────────────────────────────────────────────────

#[test]
fn changed_documents_exit_1_with_a_schema_valid_report() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[OLD, NEW, "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).ends_with("}\n"));
    assert_eq!(
        stdout(&output).lines().count(),
        1,
        "one JSON object on one line"
    );

    let report = json(&output);
    assert_matches_schema(&report);
    assert_eq!(report["version"], "redline.report.v1");
    assert_eq!(report["tool"], "redline");
    assert_eq!(report["outcome"], "CHANGES_FOUND");
    assert_eq!(report["mode"], "html-text");
    assert_eq!(report["refusal"], Value::Null);
    assert_eq!(report["summary"]["semantic_hunks"], 2);
    assert_eq!(report["summary"]["numeric_changed_hunks"], 1);
    assert_eq!(report["summary"]["clause_added_hunks"], 1);
}

#[test]
fn spans_in_the_report_slice_the_files_on_disk() {
    let sandbox = Sandbox::new();
    let report = json(&sandbox.run(&[OLD, NEW, "--json"]));
    let old = std::fs::read(OLD).unwrap();
    let new = std::fs::read(NEW).unwrap();
    let slice = |bytes: &[u8], span: &Value| {
        let start = span[0].as_u64().unwrap() as usize;
        let end = span[1].as_u64().unwrap() as usize;
        String::from_utf8(bytes[start..end].to_vec()).unwrap()
    };
    let hunks = report["hunks"].as_array().unwrap();
    let salary = hunks
        .iter()
        .find(|hunk| hunk["class"] == "numeric_changed")
        .unwrap();
    assert_eq!(slice(&old, &salary["old_span"]), "450,000");
    assert_eq!(slice(&new, &salary["new_span"]), "475,000");
    let bonus = hunks
        .iter()
        .find(|hunk| hunk["class"] == "clause_added")
        .unwrap();
    assert_eq!(slice(&old, &bonus["old_span"]), "");
    assert_eq!(
        slice(&new, &bonus["new_span"]),
        " Ms. Avery will also receive a one-time signing bonus of $100,000."
    );
    assert_eq!(bonus["numeric"], true);
}

#[test]
fn identical_and_restyled_documents_exit_0() {
    let sandbox = Sandbox::new();
    let same = sandbox.run(&[OLD, OLD, "--json"]);
    assert_eq!(same.status.code(), Some(0));
    assert_eq!(json(&same)["outcome"], "NO_SEMANTIC_CHANGE");
    assert_eq!(json(&same)["hunks"].as_array().unwrap().len(), 0);

    // Same text, different markup, spacing and entity spelling.
    let old = std::fs::read_to_string(OLD).unwrap();
    let restyled = old
        .replace("<p>", "<p class=\"x\" style='margin:1em'>\n  ")
        .replace("&nbsp;", "&#160;")
        .replace("<b>", "<strong>")
        .replace("</b>", "</strong>")
        .replace(
            "<!-- page 1 -->",
            "<script>track('<p>new text</p>')</script>",
        );
    assert_ne!(old, restyled);
    let restyled_path = sandbox.write("restyled.htm", restyled.as_bytes());
    let output = sandbox.run(&[OLD, &restyled_path, "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let report = json(&output);
    assert_matches_schema(&report);
    assert_eq!(report["outcome"], "NO_SEMANTIC_CHANGE");
    assert_eq!(report["summary"]["semantic_hunks"], 0);
    assert!(report["summary"]["formatting_only_hunks"].as_u64().unwrap() >= 1);

    // In raw mode the same pair is a change: every byte counts.
    let raw = sandbox.run(&[OLD, &restyled_path, "--json", "--mode", "raw"]);
    assert_eq!(raw.status.code(), Some(1));
}

#[test]
fn exit_zero_on_change_masks_only_the_change_verdict() {
    let sandbox = Sandbox::new();
    let changed = sandbox.run(&[OLD, NEW, "--json", "--exit-zero-on-change"]);
    assert_eq!(changed.status.code(), Some(0));
    assert_eq!(json(&changed)["outcome"], "CHANGES_FOUND");

    let refused = sandbox.run(&["/nonexistent/a", NEW, "--json", "--exit-zero-on-change"]);
    assert_eq!(refused.status.code(), Some(2));
}

#[test]
fn human_output_names_the_verdict_and_stays_redacted_by_default() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[OLD, NEW]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(
        text.starts_with("REDLINE\n\nCHANGES FOUND\n\nCompared: "),
        "{text}"
    );
    assert!(text.contains("numeric_changed"));
    assert!(text.contains("Text is redacted."));
    assert!(!text.contains("475,000"));

    let explicit = stdout(&sandbox.run(&[OLD, NEW, "--explicit"]));
    assert!(
        explicit.contains("  - 450,000\n  + 475,000\n"),
        "{explicit}"
    );
    assert!(!explicit.contains("Text is redacted."));
}

// ── Refusals ────────────────────────────────────────────────────────────────

#[test]
fn refusals_are_json_on_stdout_or_text_on_stderr_and_exit_2() {
    let sandbox = Sandbox::new();
    let pdf = sandbox.write("report.pdf", b"%PDF-1.7\n\x00\x01\x02binary");
    let empty = sandbox.write("empty.txt", b"");
    let text = sandbox.write("plain.txt", b"just some plain text\n");
    let cases: [(&[&str], &str); 5] = [
        (&["/nonexistent/old.htm", NEW], "E_IO"),
        (&[&pdf, NEW], "E_BINARY_INPUT"),
        (&[&empty, NEW], "E_EMPTY_INPUT"),
        (&[&text, NEW], "E_MODE_AMBIGUOUS"),
        (&[OLD, NEW, "--mode", "markdown"], "E_MODE_UNSUPPORTED"),
    ];
    for (args, code) in cases {
        let mut json_args = args.to_vec();
        json_args.push("--json");
        let output = sandbox.run(&json_args);
        assert_eq!(output.status.code(), Some(2), "{code}");
        assert!(output.stderr.is_empty(), "{code}: {}", stderr(&output));
        let report = json(&output);
        assert_matches_schema(&report);
        assert_eq!(report["version"], "redline.report.v1");
        assert_eq!(report["outcome"], "REFUSAL");
        assert_eq!(report["refusal"]["code"], code);

        let human = sandbox.run(args);
        assert_eq!(human.status.code(), Some(2), "{code}");
        assert!(
            human.stdout.is_empty(),
            "{code}: refusal text belongs on stderr"
        );
        let text = stderr(&human);
        assert!(
            text.starts_with(&format!("REDLINE ERROR ({code})\n")),
            "{text}"
        );
        assert!(text.contains(&format!("Reason ({code}): ")));
        assert!(text.contains("\nNext: "));
    }
}

#[test]
fn refusal_hints_are_runnable() {
    let sandbox = Sandbox::new();
    let text = sandbox.write("plain.txt", b"<p>Rate is 5.25%.</p>\n");
    let report = json(&sandbox.run(&[&text, NEW, "--json"]));
    let next = report["refusal"]["next_command"].as_str().unwrap();
    assert!(next.ends_with("--mode html-text"), "{next}");

    // Run the hint as given.
    let parts: Vec<&str> = next.split(' ').skip(1).collect();
    let rerun = sandbox.run(&parts);
    assert_eq!(rerun.status.code(), Some(1), "{}", stderr(&rerun));
}

#[test]
fn usage_errors_exit_2_on_stderr() {
    let sandbox = Sandbox::new();
    for args in [&[][..], &[OLD][..], &["--bogus"][..]] {
        let output = sandbox.run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        assert!(stderr(&output).contains("error:"), "{}", stderr(&output));
    }
    assert!(
        sandbox.ledger_records().is_empty(),
        "usage errors are not runs"
    );
}

// ── Witness ─────────────────────────────────────────────────────────────────

#[test]
fn each_run_appends_one_chained_record() {
    let sandbox = Sandbox::new();
    let first = sandbox.run(&[OLD, NEW, "--json"]);
    sandbox.run(&[OLD, OLD]);
    sandbox.run(&["/nonexistent/a", NEW, "--json"]);

    let records = sandbox.ledger_records();
    assert_eq!(records.len(), 3);
    let outcomes: Vec<&str> = records
        .iter()
        .map(|r| r["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(outcomes, ["CHANGES_FOUND", "NO_SEMANTIC_CHANGE", "REFUSAL"]);
    let exit_codes: Vec<u64> = records
        .iter()
        .map(|r| r["exit_code"].as_u64().unwrap())
        .collect();
    assert_eq!(exit_codes, [1, 0, 2]);

    assert_eq!(records[0]["prev"], Value::Null);
    assert_eq!(records[1]["prev"], records[0]["id"]);
    assert_eq!(records[2]["prev"], records[1]["id"]);

    let record = &records[0];
    assert_eq!(record["tool"], "redline");
    assert_eq!(record["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(record["inputs"][0]["path"], OLD);
    let old_digest = format!(
        "blake3:{}",
        blake3::hash(&std::fs::read(OLD).unwrap()).to_hex()
    );
    assert_eq!(record["inputs"][0]["hash"], old_digest.as_str());
    let output_hash = format!("blake3:{}", blake3::hash(&first.stdout).to_hex());
    assert_eq!(record["output_hash"], output_hash.as_str());

    // The id is the hash of the record with an empty id, keys sorted.
    let line = std::fs::read_to_string(sandbox.ledger()).unwrap();
    let line = line.lines().next().unwrap();
    let id = record["id"].as_str().unwrap();
    let unsealed = line.replacen(id, "", 1);
    assert_eq!(
        id,
        format!("blake3:{}", blake3::hash(unsealed.as_bytes()).to_hex())
    );
}

#[test]
fn no_witness_leaves_no_ledger_and_the_same_exit_code() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[OLD, NEW, "--json", "--no-witness"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!sandbox.ledger().exists());
}

#[test]
fn a_witness_failure_changes_neither_output_nor_exit_code() {
    let sandbox = Sandbox::new();
    let baseline = sandbox.run(&[OLD, NEW, "--json"]);
    let blocked = sandbox
        .command()
        .env("EPISTEMIC_WITNESS", "/dev/null/impossible/witness.jsonl")
        .args([OLD, NEW, "--json"])
        .output()
        .unwrap();
    assert_eq!(blocked.status.code(), baseline.status.code());
    assert_eq!(blocked.stdout, baseline.stdout);
    assert!(stderr(&blocked).starts_with("redline: witness: "));
}

#[test]
fn witness_timestamps_follow_source_date_epoch() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .args([OLD, NEW, "--json"])
        .output()
        .unwrap();
    assert_eq!(sandbox.ledger_records()[0]["ts"], "2023-11-14T22:13:20Z");
}

#[test]
fn witness_queries_read_the_ledger() {
    let sandbox = Sandbox::new();
    let empty = sandbox.run(&["witness", "query", "--json"]);
    assert_eq!(empty.status.code(), Some(0));
    assert_eq!(stdout(&empty), "[]\n");
    assert_eq!(sandbox.run(&["witness", "last"]).status.code(), Some(1));

    sandbox.run(&[OLD, NEW, "--json"]);
    sandbox.run(&[OLD, OLD, "--json"]);

    let all = json(&sandbox.run(&["witness", "query", "--json"]));
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert_eq!(all[0]["outcome"], "NO_SEMANTIC_CHANGE", "newest first");

    let changed = json(&sandbox.run(&["witness", "query", "--outcome", "CHANGES_FOUND", "--json"]));
    assert_eq!(changed.as_array().unwrap().len(), 1);
    let other_tool = sandbox.run(&["witness", "query", "--tool", "rvl", "--json"]);
    assert_eq!(stdout(&other_tool), "[]\n");

    let count = json(&sandbox.run(&["witness", "count", "--json"]));
    assert_eq!(count["count"], 2);
    let last = json(&sandbox.run(&["witness", "last", "--json"]));
    assert_eq!(last["outcome"], "NO_SEMANTIC_CHANGE");

    // Queries are read-only.
    assert_eq!(sandbox.ledger_records().len(), 2);
}

// ── Zero retention ──────────────────────────────────────────────────────────

#[test]
fn default_runs_leak_no_document_text_on_any_channel() {
    let sandbox = Sandbox::new();
    let old = sandbox.write(
        "a.htm",
        b"<html><body><p>CANARYALPHA owes CANARYBETA 7731 dollars.</p></body></html>",
    );
    let new = sandbox.write(
        "b.htm",
        b"<html><body><p>CANARYGAMMA owes CANARYBETA 9942 dollars. CANARYDELTA agrees.</p></body></html>",
    );
    let canaries = [
        "CANARYALPHA",
        "CANARYBETA",
        "CANARYGAMMA",
        "CANARYDELTA",
        "7731",
        "9942",
    ];
    let leaked = |haystack: &str| -> Vec<&str> {
        canaries
            .iter()
            .copied()
            .filter(|canary| haystack.contains(canary))
            .collect()
    };

    for args in [
        &[old.as_str(), new.as_str(), "--json"][..],
        &[old.as_str(), new.as_str()][..],
    ] {
        let output = sandbox.run(args);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(leaked(&stdout(&output)), Vec::<&str>::new());
        assert_eq!(leaked(&stderr(&output)), Vec::<&str>::new());
    }

    // The same run with --explicit does show them, so the check above is live.
    let explicit = sandbox.run(&[&old, &new, "--json", "--explicit"]);
    assert_eq!(
        leaked(&stdout(&explicit)),
        ["CANARYALPHA", "CANARYGAMMA", "CANARYDELTA", "7731", "9942"]
    );

    // The witness ledger never carries text, explicit or not.
    let ledger = std::fs::read_to_string(sandbox.ledger()).unwrap();
    assert_eq!(ledger.lines().count(), 3);
    assert_eq!(leaked(&ledger), Vec::<&str>::new());
}

// ── Discovery and doctor ────────────────────────────────────────────────────

#[test]
fn discovery_surfaces_are_read_only_and_consistent() {
    let sandbox = Sandbox::new();
    let describe = sandbox.run(&["--describe"]);
    let capabilities = sandbox.run(&["capabilities", "--json"]);
    assert_eq!(describe.status.code(), Some(0));
    assert_eq!(describe.stdout, capabilities.stdout);
    assert_eq!(json(&describe)["tool_version"], env!("CARGO_PKG_VERSION"));

    let version = sandbox.run(&["--version"]);
    assert_eq!(
        stdout(&version),
        format!("redline {}\n", env!("CARGO_PKG_VERSION"))
    );

    let schema = sandbox.run(&["--schema"]);
    assert_eq!(json(&schema)["title"], "redline.report.v1");

    let guide = sandbox.run(&["robot-docs", "guide"]);
    assert!(stdout(&guide).starts_with("# redline robot-docs guide\n"));
    assert_eq!(guide.stdout, sandbox.run(&["doctor", "robot-docs"]).stdout);

    let triage = sandbox.run(&["--robot-triage"]);
    assert_eq!(
        triage.stdout,
        sandbox.run(&["doctor", "triage", "--json"]).stdout
    );
    assert_eq!(json(&triage)["mode"], "triage");

    assert!(
        !sandbox.ledger().exists(),
        "discovery must not write the ledger"
    );
}

#[test]
fn every_refusal_code_in_the_guide_and_contract_is_real() {
    let sandbox = Sandbox::new();
    let contract = json(&sandbox.run(&["--describe"]));
    let declared: Vec<&str> = contract["refusals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|refusal| refusal["code"].as_str().unwrap())
        .collect();
    let in_code: Vec<&str> = redline::RefusalCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    assert_eq!(declared, in_code);
    let guide = stdout(&sandbox.run(&["robot-docs", "guide"]));
    for code in in_code {
        assert!(guide.contains(code), "guide omits {code}");
    }
}

#[test]
fn doctor_reports_an_unusable_ledger_as_unhealthy_exit_1() {
    let sandbox = Sandbox::new();
    let healthy = sandbox.run(&["doctor", "--json"]);
    assert_eq!(healthy.status.code(), Some(0));
    assert_eq!(json(&healthy)["healthy"], true);

    let as_directory: &Path = sandbox.dir.path();
    let unhealthy = sandbox
        .command()
        .env("EPISTEMIC_WITNESS", as_directory)
        .args(["doctor", "triage", "--json"])
        .output()
        .unwrap();
    assert_eq!(unhealthy.status.code(), Some(1));
    let report = json(&unhealthy);
    assert_eq!(report["healthy"], false);
    let finding = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["severity"] == "error")
        .unwrap();
    assert_eq!(finding["id"], "witness-ledger");
    assert!(
        report["next_actions"][0]
            .as_str()
            .unwrap()
            .contains("EPISTEMIC_WITNESS")
    );

    // The finding is true: a comparison run really cannot record there.
    let run = sandbox
        .command()
        .env("EPISTEMIC_WITNESS", as_directory)
        .args([OLD, NEW, "--json"])
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(1));
    assert!(stderr(&run).starts_with("redline: witness: "));
}

#[test]
fn doctor_fix_refuses_on_stderr_only() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["doctor", "--fix"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).starts_with("redline doctor --fix is unavailable"));
}
