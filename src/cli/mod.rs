//! The command-line tool: paths, output formatting, witness records and the
//! agent discovery surfaces. All comparison logic lives in the library.

pub mod args;
mod doctor;
mod human;
mod witness;

use std::io::Write;
use std::path::Path;

use clap::{CommandFactory, FromArgMatches};
use serde_json::json;

use crate::engine::{Input, ModeChoice, Options, compare};
use crate::refusal::{Refusal, RefusalCode};
use crate::report::{InputInfo, Inputs, Outcome, Report};

use args::{Args, Command, RobotDocsAction};

/// The operator contract, compiled in. `--describe` and `capabilities --json`
/// print it verbatim.
pub(crate) const OPERATOR_JSON: &str = include_str!("../../operator.json");

/// JSON Schema for `redline.report.v1`, compiled in for `--schema`.
const REPORT_SCHEMA: &str = include_str!("../../docs/redline.report.v1.schema.json");

/// Run the tool with the process arguments. Returns the exit code.
pub fn run() -> u8 {
    match dispatch(std::env::args_os().collect()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("redline: {error}");
            2
        }
    }
}

fn dispatch(argv: Vec<std::ffi::OsString>) -> std::io::Result<u8> {
    // These three work with no other arguments and win over everything else.
    // Everything after a bare `--` is a file name, whatever it looks like.
    let has = |flag: &str| {
        argv.iter()
            .skip(1)
            .take_while(|arg| *arg != "--")
            .any(|arg| arg == flag)
    };
    if has("--version") {
        println!("redline {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if has("--describe") {
        println!("{OPERATOR_JSON}");
        return Ok(0);
    }
    if has("--schema") {
        print!("{REPORT_SCHEMA}");
        return Ok(0);
    }

    let mut command = Args::command();
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
        || std::env::var_os("TERM").is_some_and(|value| value == "dumb");
    if no_color {
        command = command.color(clap::ColorChoice::Never);
    }
    let args = match command
        .try_get_matches_from(argv)
        .and_then(|matches| Args::from_arg_matches(&matches))
    {
        Ok(args) => args,
        Err(error) => {
            let code = match error.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
                _ => 2,
            };
            error.print()?;
            return Ok(code);
        }
    };

    if args.robot_triage {
        return doctor::emit_robot_triage();
    }
    match &args.command {
        Some(Command::Witness { action }) => return witness::run(action),
        Some(Command::Capabilities(flag)) => return doctor::emit_capabilities(flag.json),
        Some(Command::RobotDocs { action }) => {
            let (None | Some(RobotDocsAction::Guide)) = action;
            return doctor::emit_robot_docs();
        }
        Some(Command::Doctor(doctor_args)) => return doctor::run(doctor_args),
        None => {}
    }

    let (Some(old_path), Some(new_path)) = (&args.old, &args.new) else {
        eprintln!("error: the following required arguments were not provided:");
        if args.old.is_none() {
            eprintln!("  <OLD>");
        }
        eprintln!("  <NEW>");
        eprintln!();
        eprintln!("Usage: redline <OLD> <NEW> [OPTIONS]");
        eprintln!();
        eprintln!("For more information, try '--help'.");
        return Ok(2);
    };
    compare_files(&args, old_path, new_path)
}

/// Read both files, compare, print the result, record the run.
fn compare_files(args: &Args, old_path: &Path, new_path: &Path) -> std::io::Result<u8> {
    let old_label = old_path.to_string_lossy().into_owned();
    let new_label = new_path.to_string_lossy().into_owned();

    let old_bytes = std::fs::read(old_path);
    let new_bytes = std::fs::read(new_path);

    let result = (|| {
        let mode = ModeChoice::parse(&args.mode)?;
        let read = |bytes: &'_ std::io::Result<Vec<u8>>, side: &str, label: &str| match bytes {
            Ok(_) => Ok(()),
            Err(error) => Err(Refusal::new(
                RefusalCode::Io,
                json!({ "input": side, "path": label, "error": error.to_string() }),
                None,
            )),
        };
        read(&old_bytes, "old", &old_label)?;
        read(&new_bytes, "new", &new_label)?;
        let (Ok(old), Ok(new)) = (&old_bytes, &new_bytes) else {
            unreachable!("both reads were checked above");
        };
        compare(
            Input {
                bytes: old,
                path: Some(&old_label),
            },
            Input {
                bytes: new,
                path: Some(&new_label),
            },
            &Options {
                mode,
                explicit: args.explicit,
            },
        )
    })();

    let info = |label: &str, bytes: &std::io::Result<Vec<u8>>| InputInfo {
        path: Some(label.to_owned()),
        bytes: bytes.as_ref().ok().map(|bytes| bytes.len() as u64),
        digest: bytes
            .as_ref()
            .ok()
            .map(|bytes| format!("blake3:{}", blake3::hash(bytes).to_hex())),
    };
    let report = result.unwrap_or_else(|refusal| {
        Report::refused(
            Inputs {
                old: info(&old_label, &old_bytes),
                new: info(&new_label, &new_bytes),
            },
            refusal,
        )
    });

    let exit_code = match report.outcome {
        Outcome::NoSemanticChange => 0,
        Outcome::ChangesFound if args.exit_zero_on_change => 0,
        Outcome::ChangesFound => 1,
        Outcome::Refusal => 2,
    };

    // JSON always goes to stdout. Human refusals go to stderr.
    let output = if args.json {
        let mut line = serde_json::to_string(&report)?;
        line.push('\n');
        line.into_bytes()
    } else {
        human::render(&report).into_bytes()
    };
    if !args.json && report.outcome == Outcome::Refusal {
        std::io::stderr().write_all(&output)?;
    } else {
        let mut stdout = std::io::stdout();
        stdout.write_all(&output)?;
        stdout.flush()?;
    }

    if !args.no_witness {
        let inputs = [&report.inputs.old, &report.inputs.new]
            .into_iter()
            .map(|input| witness::WitnessInput {
                path: input.path.clone().unwrap_or_default(),
                hash: input.digest.clone(),
                bytes: input.bytes,
            })
            .collect();
        let facts = witness::RunFacts {
            inputs,
            params: json!({
                "mode": args.mode,
                "resolved_mode": report.mode.map(crate::canon::Mode::as_str),
                "explicit": args.explicit,
                "json": args.json,
            }),
            outcome: report.outcome.as_str(),
            exit_code,
            output,
        };
        // A witness failure never changes the verdict or the exit code.
        if let Err(error) = witness::record_run(facts) {
            eprintln!("redline: witness: {error}");
        }
    }

    Ok(exit_code)
}
