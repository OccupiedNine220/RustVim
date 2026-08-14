use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const CONFIG_ENV_VAR: &str = "RUSTVIM_CONFIG";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AppConfig {
    pub editor: EditorConfig,
    pub customization: CustomizationConfig,
    pub autocorrect: AutocorrectConfig,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct EditorConfig {
    pub line_numbers: bool,
    pub syntax_highlighting: bool,
    pub alternate_buffer: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            line_numbers: true,
            syntax_highlighting: true,
            alternate_buffer: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct CustomizationConfig {
    pub theme: Theme,
    pub cursor_marker: String,
    pub selected_marker: String,
    pub empty_line_marker: String,
}

impl Default for CustomizationConfig {
    fn default() -> Self {
        Self {
            theme: Theme::White,
            cursor_marker: String::from("> "),
            selected_marker: String::from(" *"),
            empty_line_marker: String::from("~"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AutocorrectConfig {
    pub enabled: bool,
    pub replacements: BTreeMap<String, String>,
}

impl Default for AutocorrectConfig {
    fn default() -> Self {
        let replacements = [
            ("teh", "the"),
            ("adn", "and"),
            ("recieve", "receive"),
            ("превет", "привет"),
            ("пожалуста", "пожалуйста"),
            ("сдесь", "здесь"),
        ]
        .into_iter()
        .map(|(from, to)| (from.to_owned(), to.to_owned()))
        .collect();
        Self {
            enabled: false,
            replacements,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    White,
    TokyoNight,
    Gruvbox,
    Catppuccin,
}

impl Theme {
    pub fn parse(value: &str) -> Option<Self> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace(['_', ' '], "-")
            .as_str()
        {
            "white" => Some(Self::White),
            "tokyo-night" | "tokyonight" => Some(Self::TokyoNight),
            "gruvbox" => Some(Self::Gruvbox),
            "catppuccin" => Some(Self::Catppuccin),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::TokyoNight => "tokyo-night",
            Self::Gruvbox => "gruvbox",
            Self::Catppuccin => "catppuccin",
        }
    }

    pub fn screen(self) -> &'static str {
        match self {
            Self::White => "\x1b[30;107m",
            Self::TokyoNight => "\x1b[38;2;192;202;245;48;2;26;27;38m",
            Self::Gruvbox => "\x1b[38;2;235;219;178;48;2;40;40;40m",
            Self::Catppuccin => "\x1b[38;2;205;214;244;48;2;30;30;46m",
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::White => "\x1b[30;47m",
            Self::TokyoNight => "\x1b[38;2;26;27;38;48;2;122;162;247m",
            Self::Gruvbox => "\x1b[38;2;40;40;40;48;2;215;153;33m",
            Self::Catppuccin => "\x1b[38;2;30;30;46;48;2;203;166;247m",
        }
    }

    pub fn keyword(self) -> &'static str {
        match self {
            Self::White => "\x1b[34;107m",
            Self::TokyoNight => "\x1b[38;2;187;154;247m",
            Self::Gruvbox => "\x1b[38;2;211;134;155m",
            Self::Catppuccin => "\x1b[38;2;203;166;247m",
        }
    }

    pub fn string(self) -> &'static str {
        match self {
            Self::White => "\x1b[32;107m",
            Self::TokyoNight => "\x1b[38;2;158;206;106m",
            Self::Gruvbox => "\x1b[38;2;184;187;38m",
            Self::Catppuccin => "\x1b[38;2;166;227;161m",
        }
    }

    pub fn number(self) -> &'static str {
        match self {
            Self::White => "\x1b[31;107m",
            Self::TokyoNight => "\x1b[38;2;255;158;100m",
            Self::Gruvbox => "\x1b[38;2;254;128;25m",
            Self::Catppuccin => "\x1b[38;2;250;179;135m",
        }
    }

    pub fn comment(self) -> &'static str {
        match self {
            Self::White => "\x1b[90;107m",
            Self::TokyoNight => "\x1b[38;2;86;95;137m",
            Self::Gruvbox => "\x1b[38;2;146;131;116m",
            Self::Catppuccin => "\x1b[38;2;108;112;134m",
        }
    }
}

fn default_true() -> bool {
    true
}

pub fn config_path() -> PathBuf {
    if let Some(path) = env::var_os(CONFIG_ENV_VAR) {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("rustvim/config.toml");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".config/rustvim/config.toml");
    }
    PathBuf::from("rustvim.toml")
}

pub fn load(path: &Path) -> io::Result<AppConfig> {
    match fs::read_to_string(path) {
        Ok(content) => toml::from_str(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(error) => Err(error),
    }
}

pub fn save(path: &Path, config: &AppConfig) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config).map_err(io::Error::other)?;
    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, Theme};

    #[test]
    fn config_round_trip_preserves_theme_and_mcp() {
        let mut config = AppConfig::default();
        config.customization.theme = Theme::Catppuccin;
        config.mcp_servers.insert(
            String::from("docs"),
            super::McpServerConfig {
                command: String::from("npx"),
                args: vec![String::from("server")],
                cwd: None,
                enabled: true,
            },
        );
        let encoded = toml::to_string(&config).expect("config should serialize");
        let decoded: AppConfig = toml::from_str(&encoded).expect("config should deserialize");
        assert_eq!(decoded.customization.theme, Theme::Catppuccin);
        assert!(decoded.mcp_servers["docs"].enabled);
    }

    #[test]
    fn theme_parser_accepts_requested_themes() {
        assert_eq!(Theme::parse("Tokyo Night"), Some(Theme::TokyoNight));
        assert_eq!(Theme::parse("tokyo-night"), Some(Theme::TokyoNight));
        assert_eq!(Theme::parse("gruvbox"), Some(Theme::Gruvbox));
        assert_eq!(Theme::parse("catppuccin"), Some(Theme::Catppuccin));
    }
}
