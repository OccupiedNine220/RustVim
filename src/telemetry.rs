use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("RUSTVIM_STATE") {
        return PathBuf::from(path).join("telemetry.jsonl");
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rustvim/telemetry.jsonl")
}

pub fn record(event: &str) -> io::Result<()> {
    let target = path();
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut file = OpenOptions::new().create(true).append(true).open(target)?;
    writeln!(
        file,
        "{{\"ts\":{timestamp},\"event\":{}}}",
        json_escape(event)
    )
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write;
                write!(escaped, "\\u{:04x}", character as u32).expect("String is writable");
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::json_escape;

    #[test]
    fn escapes_json_control_characters() {
        assert_eq!(
            json_escape("quote\" newline\n tab\t"),
            "\"quote\\\" newline\\n tab\\t\""
        );
    }
}
