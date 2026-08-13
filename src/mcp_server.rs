use std::{
    fs,
    io::{self, BufRead, Write},
};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let id = json_id(&line);
        let response = if line.contains(r#""method":"initialize""#)
            || line.contains(r#""method": "initialize""#)
        {
            initialize_response(id.as_deref())
        } else if line.contains(r#""method":"tools/list""#)
            || line.contains(r#""method": "tools/list""#)
        {
            tools_list_response(id.as_deref())
        } else if line.contains(r#""method":"tools/call""#)
            || line.contains(r#""method": "tools/call""#)
        {
            tools_call_response(id.as_deref(), &line)
        } else if line.contains(r#""method":"notifications/initialized""#)
            || line.contains(r#""method": "notifications/initialized""#)
        {
            continue;
        } else {
            error_response(id.as_deref(), -32601, "Method not found")
        };

        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }

    Ok(())
}

fn initialize_response(id: Option<&str>) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":"2024-11-05","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"rustvim-mcp","version":"0.1.0"}}}}}}"#,
        id.unwrap_or("null")
    )
}

fn tools_list_response(id: Option<&str>) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"tools":[{{"name":"read_file","description":"Read a UTF-8 text file from the RustVim workspace.","inputSchema":{{"type":"object","properties":{{"path":{{"type":"string"}}}},"required":["path"]}}}},{{"name":"editor_help","description":"Return RustVim command help.","inputSchema":{{"type":"object","properties":{{}}}}}}]}}}}"#,
        id.unwrap_or("null")
    )
}

fn tools_call_response(id: Option<&str>, request: &str) -> String {
    if request.contains(r#""name":"editor_help""#) || request.contains(r#""name": "editor_help""#) {
        return content_response(
            id,
            "RustVim supports i/a/I/A/o/O, arrows, visual line selection, y/d/p, search, substitute, :term, :set number, :set altbuffer, and :w.",
        );
    }

    if request.contains(r#""name":"read_file""#) || request.contains(r#""name": "read_file""#) {
        let Some(path) = json_string_field(request, "path") else {
            return error_response(id, -32602, "read_file requires a path");
        };

        return match fs::read_to_string(&path) {
            Ok(content) => content_response(id, &content),
            Err(error) => error_response(id, -32000, &format!("read_file failed: {error}")),
        };
    }

    error_response(id, -32602, "Unknown tool")
}

fn content_response(id: Option<&str>, text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
        id.unwrap_or("null"),
        json_escape(text)
    )
}

fn error_response(id: Option<&str>, code: i32, message: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":{},"message":"{}"}}}}"#,
        id.unwrap_or("null"),
        code,
        json_escape(message)
    )
}

fn json_id(input: &str) -> Option<String> {
    let id_key = input.find(r#""id""#)?;
    let after_key = &input[id_key + 4..];
    let colon = after_key.find(':')?;
    let value = after_key[colon + 1..].trim_start();

    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(format!(r#""{}""#, json_escape(&rest[..end])));
    }

    let end = value
        .find(|ch: char| ch == ',' || ch == '}' || ch.is_whitespace())
        .unwrap_or(value.len());
    Some(value[..end].to_owned())
}

fn json_string_field(input: &str, field: &str) -> Option<String> {
    let key = format!(r#""{field}""#);
    let key_start = input.find(&key)?;
    let after_key = &input[key_start + key.len()..];
    let colon = after_key.find(':')?;
    let value = after_key[colon + 1..].trim_start();
    let rest = value.strip_prefix('"')?;
    let mut result = String::new();
    let mut chars = rest.chars();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(result),
            '\\' => match chars.next()? {
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                '/' => result.push('/'),
                'n' => result.push('\n'),
                'r' => result.push('\r'),
                't' => result.push('\t'),
                other => result.push(other),
            },
            other => result.push(other),
        }
    }

    None
}

fn json_escape(input: &str) -> String {
    input
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            ch if ch.is_control() => ' '.to_string().chars().collect(),
            ch => vec![ch],
        })
        .collect()
}
