//! Witness ledger: one appended record per comparison run.
//!
//! The ledger is shared by the whole spine. Records follow the family
//! `witness.v0` shape, with `prev` holding the id of the record before it so
//! the file forms a hash chain.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::args::{WitnessAction, WitnessFilter};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WitnessInput {
    pub path: String,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WitnessRecord {
    #[serde(default)]
    pub id: String,
    pub tool: String,
    pub version: String,
    #[serde(default)]
    pub binary_hash: String,
    #[serde(default)]
    pub inputs: Vec<WitnessInput>,
    #[serde(default)]
    pub params: Value,
    pub outcome: String,
    pub exit_code: u8,
    #[serde(default)]
    pub output_hash: String,
    #[serde(default)]
    pub prev: Option<String>,
    pub ts: String,
}

impl WitnessRecord {
    /// Set `id` to the hash of the record with an empty id.
    fn seal(&mut self) {
        self.id.clear();
        self.id = format!(
            "blake3:{}",
            blake3::hash(canonical_json(self).as_bytes()).to_hex()
        );
    }
}

/// One line of JSON with keys sorted at every level. Sorting is done here
/// and not left to `serde_json::Map`, whose order depends on a feature flag
/// any dependency can switch on.
fn canonical_json(record: &WitnessRecord) -> String {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            scalar => out.push_str(&scalar.to_string()),
        }
    }
    let value = serde_json::to_value(record).unwrap_or(Value::Null);
    let mut out = String::new();
    write(&value, &mut out);
    out
}

/// Ledger path: `EPISTEMIC_WITNESS` when set and non-empty, otherwise
/// `~/.cmdrvl/state/witness/witness.jsonl`.
pub fn ledger_path() -> PathBuf {
    if let Some(path) = env::var_os("EPISTEMIC_WITNESS").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    let root = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .map_or_else(
            || PathBuf::from(".cmdrvl"),
            |home| PathBuf::from(home).join(".cmdrvl"),
        );
    root.join("state").join("witness").join("witness.jsonl")
}

/// What a run contributes to its witness record.
pub struct RunFacts {
    pub inputs: Vec<WitnessInput>,
    pub params: Value,
    pub outcome: &'static str,
    pub exit_code: u8,
    /// The exact bytes written to stdout or stderr as the run's result.
    pub output: Vec<u8>,
}

/// Append one record for a finished run.
pub fn record_run(facts: RunFacts) -> std::io::Result<()> {
    let path = ledger_path();
    let mut record = WitnessRecord {
        id: String::new(),
        tool: "redline".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        binary_hash: binary_hash().unwrap_or_default(),
        inputs: facts.inputs,
        params: facts.params,
        outcome: facts.outcome.to_owned(),
        exit_code: facts.exit_code,
        output_hash: format!("blake3:{}", blake3::hash(&facts.output).to_hex()),
        prev: last_id(&path),
        ts: timestamp(),
    };
    record.seal();

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", canonical_json(&record))?;
    file.sync_all()
}

fn binary_hash() -> Option<String> {
    let bytes = fs::read(env::current_exe().ok()?).ok()?;
    Some(format!("blake3:{}", blake3::hash(&bytes).to_hex()))
}

/// Id of the last record in the ledger, whichever tool wrote it.
fn last_id(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let last = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .last()?;
    let value: Value = serde_json::from_str(&last).ok()?;
    value.get("id")?.as_str().map(str::to_owned)
}

/// UTC time as `YYYY-MM-DDTHH:MM:SSZ`, from `SOURCE_DATE_EPOCH` when set.
fn timestamp() -> String {
    let seconds = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|elapsed| elapsed.as_secs())
        })
        .unwrap_or(0);
    format_utc(seconds)
}

fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Civil date from a day count (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Records in the ledger that parse, oldest first. Lines written in a shape
/// this tool does not know are skipped, not treated as errors.
fn read_records() -> std::io::Result<Vec<WitnessRecord>> {
    let file = match File::open(ledger_path()) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        if let Ok(record) = serde_json::from_str::<WitnessRecord>(&line?) {
            records.push(record);
        }
    }
    Ok(records)
}

fn matches(record: &WitnessRecord, filter: &WitnessFilter) -> bool {
    filter.tool.as_ref().is_none_or(|tool| &record.tool == tool)
        && filter
            .outcome
            .as_ref()
            .is_none_or(|outcome| &record.outcome == outcome)
        && filter
            .since
            .as_ref()
            .is_none_or(|since| record.ts.as_str() >= since.as_str())
        && filter
            .until
            .as_ref()
            .is_none_or(|until| record.ts.as_str() <= until.as_str())
        && filter.input_hash.as_ref().is_none_or(|hash| {
            record.inputs.iter().any(|input| {
                input
                    .hash
                    .as_deref()
                    .is_some_and(|have| have.contains(hash.as_str()))
            })
        })
}

/// `redline witness <query|last|count>`. Read-only.
pub fn run(action: &WitnessAction) -> std::io::Result<u8> {
    let records = read_records()?;
    match action {
        WitnessAction::Query(args) => {
            let mut found: Vec<&WitnessRecord> = records
                .iter()
                .filter(|record| matches(record, &args.filter))
                .collect();
            // Newest first, capped.
            found.reverse();
            found.truncate(args.limit);
            if args.json {
                println!("{}", serde_json::to_string(&found)?);
            } else if found.is_empty() {
                println!("No witness records match.");
            } else {
                println!(
                    "{:<18}  {:<20}  {:<20}  Inputs",
                    "ID (short)", "Timestamp", "Outcome"
                );
                for record in found {
                    let short: String = record.id.chars().take(18).collect();
                    let inputs: Vec<&str> = record
                        .inputs
                        .iter()
                        .map(|input| input.path.as_str())
                        .collect();
                    println!(
                        "{short:<18}  {:<20}  {:<20}  {}",
                        record.ts,
                        record.outcome,
                        inputs.join(" -> ")
                    );
                }
            }
            Ok(0)
        }
        WitnessAction::Count(args) => {
            let count = records
                .iter()
                .filter(|record| matches(record, &args.filter))
                .count();
            if args.json {
                println!("{}", serde_json::json!({ "count": count }));
            } else {
                println!("{count}");
            }
            Ok(0)
        }
        WitnessAction::Last(args) => match records.last() {
            Some(record) if args.json => {
                println!("{}", canonical_json(record));
                Ok(0)
            }
            Some(record) => {
                println!(
                    "{}  {}  {}  {}",
                    record.id, record.ts, record.tool, record.outcome
                );
                Ok(0)
            }
            None => {
                eprintln!("redline: witness ledger is empty");
                Ok(1)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn canonical_json_sorts_keys_and_id_covers_the_record() {
        let mut record = WitnessRecord {
            id: String::new(),
            tool: "redline".into(),
            version: "0.0.0".into(),
            binary_hash: String::new(),
            inputs: vec![],
            params: serde_json::json!({ "zeta": 1, "alpha": { "b": 1, "a": 2 } }),
            outcome: "CHANGES_FOUND".into(),
            exit_code: 1,
            output_hash: "blake3:00".into(),
            prev: None,
            ts: "2024-01-01T00:00:00Z".into(),
        };
        let json = canonical_json(&record);
        assert!(json.starts_with("{\"binary_hash\":"));
        assert!(json.contains("\"params\":{\"alpha\":{\"a\":2,\"b\":1},\"zeta\":1}"));
        record.seal();
        let first = record.id.clone();
        record.outcome = "NO_SEMANTIC_CHANGE".into();
        record.seal();
        assert_ne!(first, record.id);
    }
}
