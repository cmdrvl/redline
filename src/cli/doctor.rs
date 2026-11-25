//! Read-only diagnostics and agent discovery surfaces.
//!
//! Nothing here reads documents, writes the witness ledger or touches the
//! network. Reports follow the family `cmdrvl.doctor.v1` shape.

use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use super::OPERATOR_JSON;
use super::args::{DoctorAction, DoctorArgs};
use super::witness::ledger_path;

const DOCTOR_SCHEMA_VERSION: &str = "cmdrvl.doctor.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorMode {
    Health,
    Capabilities,
    Triage,
}

impl DoctorMode {
    fn as_str(self) -> &'static str {
        match self {
            DoctorMode::Health => "health",
            DoctorMode::Capabilities => "capabilities",
            DoctorMode::Triage => "triage",
        }
    }
}

pub fn run(args: &DoctorArgs) -> std::io::Result<u8> {
    if args.fix {
        return fix_unavailable();
    }
    if args.robot_triage {
        return emit_report(DoctorMode::Triage);
    }
    match &args.action {
        Some(DoctorAction::Health(flag)) if flag.json || args.json => {
            emit_report(DoctorMode::Health)
        }
        Some(DoctorAction::Health(_)) => {
            let report = build_report(DoctorMode::Health);
            println!(
                "redline doctor {}: {} checks passed, {} findings",
                report.status(),
                report.passed(),
                report.error_count()
            );
            Ok(report.exit_code())
        }
        Some(DoctorAction::Capabilities(flag)) if flag.json || args.json => {
            emit_report(DoctorMode::Capabilities)
        }
        Some(DoctorAction::Capabilities(_)) => emit_capabilities(false),
        Some(DoctorAction::Triage(flag)) if flag.json || args.json => {
            emit_report(DoctorMode::Triage)
        }
        Some(DoctorAction::Triage(_)) => human_triage(),
        Some(DoctorAction::RobotDocs) => emit_robot_docs(),
        None if args.json => emit_report(DoctorMode::Health),
        None => human_triage(),
    }
}

fn human_triage() -> std::io::Result<u8> {
    let report = build_report(DoctorMode::Triage);
    println!("REDLINE DOCTOR");
    println!();
    println!("Status: {}", report.status());
    println!("Checks passed: {}", report.passed());
    println!("Findings: {}", report.error_count());
    if report.error_count() > 0 {
        println!();
        for check in report.checks.iter().filter(|check| !check.pass) {
            println!("- {}: {}", check.id, check.message);
            println!("  next: {}", check.fix);
        }
    }
    println!();
    println!("Next: redline capabilities --json");
    Ok(report.exit_code())
}

/// `redline capabilities --json` is the operator.v1 contract, byte-identical
/// to `--describe`.
pub fn emit_capabilities(json: bool) -> std::io::Result<u8> {
    if json {
        println!("{OPERATOR_JSON}");
    } else {
        let contract = operator_contract();
        println!("redline capabilities");
        println!(
            "schema_version: {}",
            contract["schema_version"].as_str().unwrap_or("?")
        );
        println!(
            "contract_version: {}",
            contract["contract_version"].as_str().unwrap_or("?")
        );
        println!("json: redline capabilities --json");
    }
    Ok(0)
}

pub fn emit_robot_docs() -> std::io::Result<u8> {
    print!("{}", include_str!("robot_docs.md"));
    Ok(0)
}

/// `redline --robot-triage`: the triage-mode report.
pub fn emit_robot_triage() -> std::io::Result<u8> {
    emit_report(DoctorMode::Triage)
}

fn emit_report(mode: DoctorMode) -> std::io::Result<u8> {
    let report = build_report(mode);
    println!("{}", serde_json::to_string_pretty(&report.to_json())?);
    Ok(report.exit_code())
}

fn fix_unavailable() -> std::io::Result<u8> {
    let mut stderr = std::io::stderr();
    writeln!(
        stderr,
        "redline doctor --fix is unavailable: diagnostics are read-only in this release."
    )?;
    writeln!(stderr, "Try triage: redline doctor triage --json")?;
    writeln!(
        stderr,
        "Try capabilities --json: redline capabilities --json"
    )?;
    writeln!(stderr, "Try robot-docs guide: redline robot-docs guide")?;
    stderr.flush()?;
    Ok(2)
}

fn operator_contract() -> Value {
    serde_json::from_str(OPERATOR_JSON).unwrap_or(Value::Null)
}

struct Check {
    id: &'static str,
    pass: bool,
    message: String,
    fix: &'static str,
}

struct Report {
    mode: DoctorMode,
    checks: Vec<Check>,
}

impl Report {
    fn error_count(&self) -> usize {
        self.checks.iter().filter(|check| !check.pass).count()
    }

    fn passed(&self) -> usize {
        self.checks.len() - self.error_count()
    }

    fn healthy(&self) -> bool {
        self.error_count() == 0
    }

    fn status(&self) -> &'static str {
        if self.healthy() {
            "healthy"
        } else {
            "unhealthy"
        }
    }

    /// Doctor contract: 0 healthy, 1 unhealthy, never 2.
    fn exit_code(&self) -> u8 {
        if self.healthy() { 0 } else { 1 }
    }

    fn to_json(&self) -> Value {
        let findings: Vec<Value> = self
            .checks
            .iter()
            .map(|check| {
                let mut finding = json!({
                    "id": check.id,
                    "severity": if check.pass { "info" } else { "error" },
                    "message": check.message,
                });
                if !check.pass {
                    finding["fix"] = json!(check.fix);
                }
                finding
            })
            .collect();

        let mut report = json!({
            "schema_version": DOCTOR_SCHEMA_VERSION,
            "mode": self.mode.as_str(),
            "tool": "redline",
            "tool_version": env!("CARGO_PKG_VERSION"),
            "healthy": self.healthy(),
            "findings": findings,
        });
        match self.mode {
            DoctorMode::Health => {}
            DoctorMode::Capabilities => report["contract"] = operator_contract(),
            DoctorMode::Triage => {
                let mut next_actions: Vec<String> = self
                    .checks
                    .iter()
                    .filter(|check| !check.pass)
                    .map(|check| check.fix.to_owned())
                    .collect();
                if next_actions.is_empty() {
                    next_actions.push("redline capabilities --json".to_owned());
                    next_actions.push("redline <old> <new> --json".to_owned());
                } else {
                    next_actions.push("redline doctor capabilities --json".to_owned());
                }
                report["next_actions"] = json!(next_actions);
            }
        }
        report
    }
}

fn build_report(mode: DoctorMode) -> Report {
    let contract = operator_contract();
    let manifest_ok =
        contract["name"] == "redline" && contract["tool_version"] == env!("CARGO_PKG_VERSION");
    let ledger = ledger_path();
    let ledger_problem = ledger_problem(&ledger);

    let checks = vec![
        Check {
            id: "binary-metadata",
            pass: true,
            message: format!("redline {} is runnable", env!("CARGO_PKG_VERSION")),
            fix: "reinstall redline",
        },
        Check {
            id: "operator-manifest",
            pass: manifest_ok,
            message: if manifest_ok {
                "compiled operator manifest is readable and matches this build".to_owned()
            } else {
                "compiled operator manifest is missing, unreadable or for another version"
                    .to_owned()
            },
            fix: "rebuild redline with a valid operator.json",
        },
        Check {
            id: "witness-ledger",
            pass: ledger_problem.is_none(),
            message: match &ledger_problem {
                None => format!("witness ledger path is usable: {}", ledger.display()),
                Some(problem) => format!("witness ledger at {} {problem}", ledger.display()),
            },
            fix: "set EPISTEMIC_WITNESS to a writable file path, or pass --no-witness",
        },
    ];
    Report { mode, checks }
}

/// Why comparison runs could not append to the ledger, judged without
/// writing anything: the ledger must be a file, or absent under a directory.
fn ledger_problem(ledger: &Path) -> Option<String> {
    if ledger.is_dir() {
        return Some("is a directory, not a file".to_owned());
    }
    if ledger.exists() {
        return None;
    }
    let mut ancestor = ledger.parent();
    while let Some(dir) = ancestor {
        if dir.as_os_str().is_empty() || dir.is_dir() {
            return None;
        }
        if dir.exists() {
            return Some(format!(
                "cannot be created: {} is not a directory",
                dir.display()
            ));
        }
        ancestor = dir.parent();
    }
    None
}
