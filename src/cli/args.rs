//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "redline",
    about = "Diff two versions of a document and report every change as exact byte spans in the original files.",
    override_usage = "redline <OLD> <NEW> [OPTIONS]\n       redline capabilities --json\n       redline robot-docs guide\n       redline witness <query|last|count> [OPTIONS]\n       redline doctor [health|capabilities|triage|robot-docs] [--json]",
    after_help = "Output is redacted by default (zero-retention safe): byte spans, classes, counts and keyed digests only, no document text.\nPass --explicit to include the changed text of each hunk.",
    subcommand_negates_reqs = true,
    disable_version_flag = true
)]
pub struct Args {
    /// The earlier version of the document.
    #[arg(value_name = "OLD")]
    pub old: Option<PathBuf>,

    /// The later version of the document.
    #[arg(value_name = "NEW")]
    pub new: Option<PathBuf>,

    /// How inputs are normalized before diffing: auto, raw, whitespace or html-text.
    #[arg(long, value_name = "MODE", default_value = "auto")]
    pub mode: String,

    /// Exit 0 instead of 1 on CHANGES_FOUND, for `set -e` pipelines. Refusals still exit 2; never affects `doctor`.
    #[arg(long)]
    pub exit_zero_on_change: bool,

    /// Emit JSON output (single object).
    #[arg(long)]
    pub json: bool,

    /// Suppress witness ledger recording.
    #[arg(long)]
    pub no_witness: bool,

    /// Include the changed text of each hunk (default: redacted for zero-retention safety).
    #[arg(long)]
    pub explicit: bool,

    /// Print compiled operator.json and exit 0.
    #[arg(long)]
    pub describe: bool,

    /// Print JSON Schema for the redline.report.v1 output format and exit 0.
    #[arg(long)]
    pub schema: bool,

    /// Print version and exit 0.
    #[arg(long)]
    pub version: bool,

    /// Emit one-call machine triage for headless agents (alias of `doctor triage --json`).
    #[arg(long = "robot-triage")]
    pub robot_triage: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Query the witness ledger.
    Witness {
        #[command(subcommand)]
        action: WitnessAction,
    },
    /// Print the machine-readable capability contract.
    Capabilities(JsonFlag),
    /// Print the agent guide.
    RobotDocs {
        #[command(subcommand)]
        action: Option<RobotDocsAction>,
    },
    /// Read-only diagnostics.
    Doctor(DoctorArgs),
}

#[derive(Debug, Clone, ClapArgs)]
pub struct JsonFlag {
    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum RobotDocsAction {
    /// The agent guide.
    Guide,
}

#[derive(Debug, Clone, ClapArgs)]
pub struct DoctorArgs {
    /// Alias of `doctor triage --json`.
    #[arg(long = "robot-triage")]
    pub robot_triage: bool,

    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,

    /// Refuse safely; repair mode is not available in this release.
    #[arg(long, hide = true)]
    pub fix: bool,

    #[command(subcommand)]
    pub action: Option<DoctorAction>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DoctorAction {
    /// Health checks.
    Health(JsonFlag),
    /// Health checks plus the operator contract.
    Capabilities(JsonFlag),
    /// Health checks plus what to run next.
    Triage(JsonFlag),
    /// The agent guide.
    RobotDocs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WitnessAction {
    /// List matching records, newest first.
    Query(WitnessQueryArgs),
    /// Show the most recent record.
    Last(JsonFlag),
    /// Count matching records.
    Count(WitnessCountArgs),
}

#[derive(Debug, Clone, ClapArgs)]
pub struct WitnessFilter {
    /// Only records written by this tool.
    #[arg(long)]
    pub tool: Option<String>,
    /// Only records at or after this UTC timestamp (YYYY-MM-DDTHH:MM:SSZ).
    #[arg(long)]
    pub since: Option<String>,
    /// Only records at or before this UTC timestamp.
    #[arg(long)]
    pub until: Option<String>,
    /// Only records with this outcome.
    #[arg(long)]
    pub outcome: Option<String>,
    /// Only records with an input whose hash contains this text.
    #[arg(long)]
    pub input_hash: Option<String>,
}

#[derive(Debug, Clone, ClapArgs)]
pub struct WitnessQueryArgs {
    #[command(flatten)]
    pub filter: WitnessFilter,
    /// Most records to return.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, ClapArgs)]
pub struct WitnessCountArgs {
    #[command(flatten)]
    pub filter: WitnessFilter,
    /// Emit JSON output.
    #[arg(long)]
    pub json: bool,
}
