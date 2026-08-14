use std::{fs::{self, OpenOptions}, io::{self, Write}, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

pub fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("RUSTVIM_STATE") { return PathBuf::from(path).join("telemetry.jsonl"); }
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))).unwrap_or_else(|| PathBuf::from("."));
    base.join("rustvim/telemetry.jsonl")
}

pub fn record(event: &str) -> io::Result<()> {
    let target = path();
    if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut file = OpenOptions::new().create(true).append(true).open(target)?;
    writeln!(file, "{{\"ts\":{timestamp},\"event\":{}}}", json_escape(event))
}

fn json_escape(value: &str) -> String { format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")) }
