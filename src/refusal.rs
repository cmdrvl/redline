//! Typed refusals. A refusal says why no verdict was produced and what to
//! run next. It is never a guess.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCode {
    Io,
    EmptyInput,
    BinaryInput,
    ModeUnsupported,
    ModeAmbiguous,
}

impl RefusalCode {
    pub const ALL: [RefusalCode; 5] = [
        RefusalCode::Io,
        RefusalCode::EmptyInput,
        RefusalCode::BinaryInput,
        RefusalCode::ModeUnsupported,
        RefusalCode::ModeAmbiguous,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            RefusalCode::Io => "E_IO",
            RefusalCode::EmptyInput => "E_EMPTY_INPUT",
            RefusalCode::BinaryInput => "E_BINARY_INPUT",
            RefusalCode::ModeUnsupported => "E_MODE_UNSUPPORTED",
            RefusalCode::ModeAmbiguous => "E_MODE_AMBIGUOUS",
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            RefusalCode::Io => "cannot read input file",
            RefusalCode::EmptyInput => "input is empty",
            RefusalCode::BinaryInput => "input is not UTF-8 text",
            RefusalCode::ModeUnsupported => "mode is not available in this release",
            RefusalCode::ModeAmbiguous => "cannot tell how to read the inputs; choose a mode",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Refusal {
    pub code: &'static str,
    pub message: &'static str,
    /// Code-specific facts about what was seen.
    pub detail: Value,
    /// A command that gets past this refusal, when one exists.
    pub next_command: Option<String>,
}

impl Refusal {
    pub fn new(code: RefusalCode, detail: Value, next_command: Option<String>) -> Self {
        Refusal {
            code: code.as_str(),
            message: code.reason(),
            detail,
            next_command,
        }
    }
}
