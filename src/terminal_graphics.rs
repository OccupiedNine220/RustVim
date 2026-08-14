use std::{env, fs, io, path::Path, process::Command};

use base64::{engine::general_purpose::STANDARD, Engine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsProtocol {
    Kitty,
    Iterm2,
    External,
    Unsupported,
}

pub fn detect_graphics_protocol() -> GraphicsProtocol {
    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    let term_program = env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_ascii_lowercase();

    if env::var_os("KITTY_WINDOW_ID").is_some() || term.contains("kitty") {
        GraphicsProtocol::Kitty
    } else if term_program.contains("iterm")
        || term_program.contains("wezterm")
        || env::var_os("WEZTERM_PANE").is_some()
    {
        GraphicsProtocol::Iterm2
    } else if command_exists("chafa") || command_exists("viu") {
        GraphicsProtocol::External
    } else {
        GraphicsProtocol::Unsupported
    }
}

pub fn render_image(path: &Path, columns: usize, rows: usize) -> io::Result<String> {
    match detect_graphics_protocol() {
        GraphicsProtocol::Kitty => {
            let encoded_path = STANDARD.encode(path.as_os_str().as_encoded_bytes());
            Ok(format!(
                "\x1b_Ga=T,t=f,f=100,c={},r={},q=2;{}\x1b\\",
                columns.max(1),
                rows.max(1),
                encoded_path
            ))
        }
        GraphicsProtocol::Iterm2 => {
            let data = STANDARD.encode(fs::read(path)?);
            let name = STANDARD.encode(path.file_name().unwrap_or_default().as_encoded_bytes());
            Ok(format!(
                "\x1b]1337;File=name={name};inline=1;width={}cols;height={}rows;preserveAspectRatio=1:{data}\x07",
                columns.max(1),
                rows.max(1)
            ))
        }
        GraphicsProtocol::External => render_external(path, columns, rows),
        GraphicsProtocol::Unsupported => Ok(String::from(
            "Image preview is unavailable: use Kitty, WezTerm, iTerm2, chafa, or viu.",
        )),
    }
}

fn render_external(path: &Path, columns: usize, rows: usize) -> io::Result<String> {
    let output = if command_exists("chafa") {
        Command::new("chafa")
            .arg("--format=symbols")
            .arg(format!("--size={}x{}", columns.max(1), rows.max(1)))
            .arg(path)
            .output()?
    } else {
        Command::new("viu")
            .arg("-w")
            .arg(columns.max(1).to_string())
            .arg("-h")
            .arg(rows.max(1).to_string())
            .arg(path)
            .output()?
    };
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Ok(format!(
            "Image renderer failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()
        .is_ok_and(|status| status.success())
}

pub fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif")
    )
}
