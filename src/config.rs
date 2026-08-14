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
    pub render_rows: Option<usize>,
    pub render_cols: Option<usize>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            line_numbers: true,
            syntax_highlighting: true,
            alternate_buffer: true,
            render_rows: None,
            render_cols: None,
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
    Dracula,
    Nord,
    OneDark,
    SolarizedDark,
    RosePine,
    Monokai,
    Everforest,
    Cyberpunk,
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
            "dracula" => Some(Self::Dracula),
            "nord" => Some(Self::Nord),
            "one-dark" | "onedark" => Some(Self::OneDark),
            "solarized-dark" | "solarized" => Some(Self::SolarizedDark),
            "rose-pine" | "rosepine" => Some(Self::RosePine),
            "monokai" => Some(Self::Monokai),
            "everforest" => Some(Self::Everforest),
            "cyberpunk" => Some(Self::Cyberpunk),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::TokyoNight => "tokyo-night",
            Self::Gruvbox => "gruvbox",
            Self::Catppuccin => "catppuccin",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::OneDark => "one-dark",
            Self::SolarizedDark => "solarized-dark",
            Self::RosePine => "rose-pine",
            Self::Monokai => "monokai",
            Self::Everforest => "everforest",
            Self::Cyberpunk => "cyberpunk",
        }
    }

    pub fn screen(self) -> &'static str {
        match self {
            Self::White => "\x1b[30;107m",
            Self::TokyoNight => "\x1b[38;2;192;202;245;48;2;26;27;38m",
            Self::Gruvbox => "\x1b[38;2;235;219;178;48;2;40;40;40m",
            Self::Catppuccin => "\x1b[38;2;205;214;244;48;2;30;30;46m",
            Self::Dracula => "\x1b[38;2;248;248;242;48;2;40;42;54m",
            Self::Nord => "\x1b[38;2;216;222;233;48;2;46;52;64m",
            Self::OneDark => "\x1b[38;2;171;178;191;48;2;40;44;52m",
            Self::SolarizedDark => "\x1b[38;2;131;148;150;48;2;0;43;54m",
            Self::RosePine => "\x1b[38;2;224;222;244;48;2;25;23;36m",
            Self::Monokai => "\x1b[38;2;248;248;242;48;2;39;40;34m",
            Self::Everforest => "\x1b[38;2;211;198;170;48;2;39;46;38m",
            Self::Cyberpunk => "\x1b[38;2;255;255;255;48;2;20;8;38m",
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::White => "\x1b[30;47m",
            Self::TokyoNight => "\x1b[38;2;26;27;38;48;2;122;162;247m",
            Self::Gruvbox => "\x1b[38;2;40;40;40;48;2;215;153;33m",
            Self::Catppuccin => "\x1b[38;2;30;30;46;48;2;203;166;247m",
            Self::Dracula => "\x1b[38;2;40;42;54;48;2;189;147;249m",
            Self::Nord => "\x1b[38;2;46;52;64;48;2;136;192;208m",
            Self::OneDark => "\x1b[38;2;40;44;52;48;2;97;175;239m",
            Self::SolarizedDark => "\x1b[38;2;0;43;54;48;2;181;137;0m",
            Self::RosePine => "\x1b[38;2;25;23;36;48;2;235;111;146m",
            Self::Monokai => "\x1b[38;2;39;40;34;48;2;166;226;46m",
            Self::Everforest => "\x1b[38;2;39;46;38;48;2;167;192;128m",
            Self::Cyberpunk => "\x1b[38;2;20;8;38;48;2;255;0;170m",
        }
    }

    pub fn keyword(self) -> &'static str {
        match self {
            Self::White => "\x1b[34;107m",
            Self::TokyoNight => "\x1b[38;2;187;154;247m",
            Self::Gruvbox => "\x1b[38;2;211;134;155m",
            Self::Catppuccin => "\x1b[38;2;203;166;247m",
            Self::Dracula => "\x1b[38;2;255;121;198m",
            Self::Nord => "\x1b[38;2;129;161;193m",
            Self::OneDark => "\x1b[38;2;198;120;221m",
            Self::SolarizedDark => "\x1b[38;2;38;139;210m",
            Self::RosePine => "\x1b[38;2;196;167;231m",
            Self::Monokai => "\x1b[38;2;249;38;114m",
            Self::Everforest => "\x1b[38;2;230;126;128m",
            Self::Cyberpunk => "\x1b[38;2;0;255;255m",
        }
    }

    pub fn string(self) -> &'static str {
        match self {
            Self::White => "\x1b[32;107m",
            Self::TokyoNight => "\x1b[38;2;158;206;106m",
            Self::Gruvbox => "\x1b[38;2;184;187;38m",
            Self::Catppuccin => "\x1b[38;2;166;227;161m",
            Self::Dracula => "\x1b[38;2;80;250;123m",
            Self::Nord => "\x1b[38;2;163;190;140m",
            Self::OneDark => "\x1b[38;2;152;195;121m",
            Self::SolarizedDark => "\x1b[38;2;133;153;0m",
            Self::RosePine => "\x1b[38;2;156;207;216m",
            Self::Monokai => "\x1b[38;2;166;226;46m",
            Self::Everforest => "\x1b[38;2;167;192;128m",
            Self::Cyberpunk => "\x1b[38;2;57;255;20m",
        }
    }

    pub fn number(self) -> &'static str {
        match self {
            Self::White => "\x1b[31;107m",
            Self::TokyoNight => "\x1b[38;2;255;158;100m",
            Self::Gruvbox => "\x1b[38;2;254;128;25m",
            Self::Catppuccin => "\x1b[38;2;250;179;135m",
            Self::Dracula => "\x1b[38;2;255;184;108m",
            Self::Nord => "\x1b[38;2;208;135;112m",
            Self::OneDark => "\x1b[38;2;209;154;102m",
            Self::SolarizedDark => "\x1b[38;2;203;75;22m",
            Self::RosePine => "\x1b[38;2;246;193;119m",
            Self::Monokai => "\x1b[38;2;230;219;116m",
            Self::Everforest => "\x1b[38;2;219;188;116m",
            Self::Cyberpunk => "\x1b[38;2;255;238;0m",
        }
    }

    pub fn comment(self) -> &'static str {
        match self {
            Self::White => "\x1b[90;107m",
            Self::TokyoNight => "\x1b[38;2;86;95;137m",
            Self::Gruvbox => "\x1b[38;2;146;131;116m",
            Self::Catppuccin => "\x1b[38;2;108;112;134m",
            Self::Dracula => "\x1b[38;2;98;114;164m",
            Self::Nord => "\x1b[38;2;76;86;106m",
            Self::OneDark => "\x1b[38;2;92;99;112m",
            Self::SolarizedDark => "\x1b[38;2;88;110;117m",
            Self::RosePine => "\x1b[38;2;110;106;134m",
            Self::Monokai => "\x1b[38;2;117;113;94m",
            Self::Everforest => "\x1b[38;2;133;146;137m",
            Self::Cyberpunk => "\x1b[38;2;128;0;255m",
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
        assert_eq!(Theme::parse("dracula"), Some(Theme::Dracula));
        assert_eq!(Theme::parse("solarized-dark"), Some(Theme::SolarizedDark));
        assert_eq!(Theme::parse("one_dark"), Some(Theme::OneDark));
    }
}
