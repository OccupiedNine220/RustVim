mod ai;
mod battle_pass;
mod config;
mod economy;
mod license;
mod plugins;
mod telemetry;
mod terminal_graphics;

use std::{
    cmp::{max, min},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

use ai::AiClient;
use battle_pass::BattlePass;
use config::{AppConfig, McpServerConfig, Theme};
use economy::Economy;
use license::{Access, LicenseState};
use plugins::PluginManager;
use terminal_graphics::{is_image_path, render_image};

const PRO_ENV_VAR: &str = "RUSTVIM_PRO";
const NITRO_ENV_VAR: &str = "RUSTVIM_NITRO";
const SUBSCRIPTION_PROMPT: &str = "Оформите подписку RustVim Pro для AI и премиум-функций.";
const LICENSE_TEXT: &[&str] = &[
    "ЛИЦЕНЗИОННОЕ СОГЛАШЕНИЕ RUSTVIM",
    "",
    "RustVim предоставляется по лицензии MIT. Вы можете использовать, копировать,",
    "изменять и распространять программу при сохранении текста лицензии и отказа",
    "от гарантий из исходного проекта.",
    "",
    "Nitro, Pro, внутриигровая валюта Terminal Tokens, слоты, лутбоксы и Battle Pass",
    "являются функциями приложения. Terminal Tokens не являются деньгами, не имеют",
    "денежной стоимости, не продаются и не обмениваются на реальные товары или деньги.",
    "Слоты и лутбоксы используют исключительно внутриигровую валюту и не принимают",
    "платежи, банковские данные или иные реальные средства.",
    "",
    "Состояние лицензии, прогресс и внутриигровые данные хранятся локально на этом ПК.",
    "AI-функции могут отправлять содержимое файла настроенному пользователем endpoint.",
    "Вы принимаете ответственность за свои данные, конфигурацию и использование ПО.",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Agreement,
    Normal,
    Insert,
    VisualLine,
    Command,
    Files,
    Image,
    MarkdownPreview,
    Welcome,
}

#[derive(Clone)]
struct TabState {
    path: PathBuf,
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    dirty: bool,
}

enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Tab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    CtrlC,
    Unknown,
}

#[derive(Clone, Copy)]
enum AiAction {
    Show,
    Insert,
    Replace,
}

struct RawTerminal;

impl RawTerminal {
    fn enter(use_alternate_buffer: bool) -> io::Result<Self> {
        run_stty(&["raw", "-echo", "min", "0", "time", "1"])?;
        if use_alternate_buffer {
            print!("\x1b[?1049h");
        }
        print!("\x1b[?25l");
        io::stdout().flush()?;
        Ok(Self)
    }

    fn suspend_for_child() -> io::Result<()> {
        run_stty(&["sane"])?;
        print!("\x1b[?25h\x1b[?1049l\x1b[0m");
        io::stdout().flush()
    }

    fn resume_from_child(use_alternate_buffer: bool) -> io::Result<()> {
        run_stty(&["raw", "-echo", "min", "0", "time", "1"])?;
        if use_alternate_buffer {
            print!("\x1b[?1049h");
        }
        print!("\x1b[?25l");
        io::stdout().flush()
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        let _ = run_stty(&["sane"]);
        print!("\x1b[?25h\x1b[?1049l\x1b[0m");
        let _ = io::stdout().flush();
    }
}

struct Editor {
    path: PathBuf,
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    mode: Mode,
    dirty: bool,
    message: String,
    command: String,
    command_prompt: char,
    clipboard: Vec<String>,
    undo: Vec<Vec<String>>,
    pending: Option<char>,
    visual_anchor: usize,
    search: Option<String>,
    show_numbers: bool,
    use_alternate_buffer: bool,
    pro_active: bool,
    syntax_enabled: bool,
    scroll_line: usize,
    browser_dir: PathBuf,
    browser_entries: Vec<PathBuf>,
    browser_selected: usize,
    image_path: Option<PathBuf>,
    config: AppConfig,
    config_path: PathBuf,
    theme: Theme,
    autocorrect_enabled: bool,
    redo: Vec<Vec<String>>,
    tabs: Vec<TabState>,
    current_tab: usize,
    key_presses: usize,
    ad_message: Option<String>,
    license_state: LicenseState,
    access: Access,
    plugins: PluginManager,
    agreement_required: bool,
    battle_pass: BattlePass,
    economy: Economy,
}

impl Editor {
    fn open(path: PathBuf) -> io::Result<Self> {
        Self::open_with_pro(path, pro_active_from_env())
    }

    fn open_welcome() -> io::Result<Self> {
        let mut editor = Self::open_with_pro(PathBuf::from("untitled.txt"), pro_active_from_env())?;
        editor.mode = if editor.agreement_required {
            Mode::Agreement
        } else {
            Mode::Welcome
        };
        editor.message = if editor.pro_active {
            String::from("RustVim Pro active. Enter: start an empty file. :help: Vim guide.")
        } else {
            String::from("Оформите RustVim Pro, чтобы открыть AI и премиум-функции.")
        };
        Ok(editor)
    }

    fn open_with_pro(path: PathBuf, pro_active: bool) -> io::Result<Self> {
        let mut license_state = license::load()?;
        let now = SystemTime::now();
        if !pro_active && license_state.trial_started_at.is_none() {
            license::start_trial(&mut license_state, now)?;
        }
        let access = license::access(&license_state, nitro_active_from_env(), pro_active, now);
        let config_path = config::config_path();
        let config = config::load(&config_path)?;
        let lines = match fs::read_to_string(&path) {
            Ok(content) => {
                let mut lines = split_editor_lines(&content);
                if lines.is_empty() {
                    lines.push(String::new());
                }
                lines
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => vec![String::new()],
            Err(error) => return Err(error),
        };

        let browser_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let (show_numbers, syntax_enabled, use_alternate_buffer, theme, autocorrect_enabled) =
            if pro_active {
                (
                    config.editor.line_numbers,
                    config.editor.syntax_highlighting,
                    config.editor.alternate_buffer,
                    config.customization.theme,
                    config.autocorrect.enabled,
                )
            } else {
                (true, false, true, Theme::White, false)
            };

        let agreement_required = !access.agreement_accepted;
        let mut editor = Self {
            path,
            lines,
            cursor_line: 0,
            cursor_col: 0,
            mode: Mode::Normal,
            dirty: false,
            message: String::from("Ready. :help for commands."),
            command: String::new(),
            command_prompt: ':',
            clipboard: Vec::new(),
            undo: Vec::new(),
            pending: None,
            visual_anchor: 0,
            search: None,
            show_numbers,
            use_alternate_buffer,
            pro_active: access.pro,
            syntax_enabled,
            scroll_line: 0,
            browser_dir,
            browser_entries: Vec::new(),
            browser_selected: 0,
            image_path: None,
            config,
            config_path,
            theme,
            autocorrect_enabled,
            redo: Vec::new(),
            tabs: Vec::new(),
            current_tab: 0,
            key_presses: 0,
            ad_message: None,
            license_state,
            agreement_required,
            access: access.clone(),
            plugins: PluginManager::new(),
            battle_pass: BattlePass::load()?,
            economy: Economy::load()?,
        };
        if editor.agreement_required {
            editor.mode = Mode::Agreement;
            editor.message = String::from(
                "Перед первым запуском примите соглашение: Y/Enter — принять, N — выйти.",
            );
        }
        Ok(editor)
    }

    fn render(&mut self) -> io::Result<()> {
        print!("{}\x1b[2J\x1b[H", self.theme.screen());
        let (terminal_rows, terminal_cols) = terminal_size();
        if self.mode == Mode::Files {
            return self.render_file_manager(terminal_rows, terminal_cols);
        }
        if self.mode == Mode::Image {
            return self.render_image_preview(terminal_rows, terminal_cols);
        }
        if self.mode == Mode::MarkdownPreview {
            return self.render_markdown_preview(terminal_rows, terminal_cols);
        }
        if self.mode == Mode::Welcome {
            return self.render_welcome(terminal_rows, terminal_cols);
        }
        if self.mode == Mode::Agreement {
            return self.render_agreement(terminal_rows, terminal_cols);
        }
        let reserved_rows = if self.pro_active && self.ad_message.is_none() {
            4
        } else {
            5
        };
        let (render_rows, render_cols) = self.render_dimensions(terminal_rows, terminal_cols);
        let viewport_rows = render_rows.saturating_sub(reserved_rows).max(1);
        self.ensure_cursor_visible(viewport_rows);
        let (sel_start, sel_end) = self.selection_bounds();
        let number_width = max(4, self.lines.len().to_string().len());
        let viewport_end = min(self.scroll_line + viewport_rows, self.lines.len());
        for index in self.scroll_line..viewport_end {
            let line = &self.lines[index];
            let selected = self.mode == Mode::VisualLine && (sel_start..=sel_end).contains(&index);
            let cursor = index == self.cursor_line;
            let marker = self.line_marker(cursor, selected);
            let rendered_line = if cursor {
                render_cursor_line(
                    line,
                    self.cursor_col,
                    syntax_for_path(&self.path).filter(|_| self.syntax_active()),
                    self.theme,
                )
            } else if self.syntax_active() {
                highlight_syntax(line, syntax_for_path(&self.path), self.theme)
            } else {
                line.to_owned()
            };
            if self.show_numbers {
                let prefix = format!("{marker} {:>width$} | ", index + 1, width = number_width);
                let available_cols = render_cols.saturating_sub(visible_width(&prefix));
                let rendered_line = truncate_terminal_line(&rendered_line, available_cols);
                print!("{}{prefix}{rendered_line}\r\n", self.theme.screen());
            } else {
                let prefix = format!("{marker} ");
                let available_cols = render_cols.saturating_sub(visible_width(&prefix));
                let rendered_line = truncate_terminal_line(&rendered_line, available_cols);
                print!("{}{prefix}{rendered_line}\r\n", self.theme.screen());
            }
        }
        for _ in viewport_end.saturating_sub(self.scroll_line)..viewport_rows {
            print!(
                "{}{}\r\n",
                self.theme.screen(),
                truncate_terminal_line(self.empty_line_marker(), render_cols)
            );
        }
        if !self.pro_active {
            print!("\x1b[33m{SUBSCRIPTION_PROMPT}\x1b[0m\r\n");
        }
        if let Some(ad) = &self.ad_message {
            print!("\x1b[33m{}\x1b[0m\r\n", ad);
        }

        let mode = match self.mode {
            Mode::Agreement => "AGREEMENT",
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::VisualLine => "VISUAL LINE",
            Mode::Command => "COMMAND",
            Mode::Files => "FILES",
            Mode::Image => "IMAGE",
            Mode::MarkdownPreview => "MARKDOWN PREVIEW",
            Mode::Welcome => "WELCOME",
        };
        print!(
            "{} {}{}  {}  line {}, col {}  theme:{}  battle-pass:{} {} {}\r\n",
            self.theme.status(),
            self.path.display(),
            if self.dirty { " [+]" } else { "" },
            mode,
            self.cursor_line + 1,
            self.cursor_col + 1,
            self.theme.name(),
            self.battle_pass.badge(),
            if self.pro_active && self.access.pro_trial_remaining > std::time::Duration::ZERO {
                format!("trial:{}m", self.access.pro_trial_remaining.as_secs() / 60)
            } else {
                String::new()
            },
            self.theme.screen(),
        );
        if self.mode == Mode::Command {
            print!("{}{}", self.command_prompt, self.command);
        } else if self.mode == Mode::Agreement {
            print!(
                "Нажмите Y/Enter для принятия, N/Esc — выход. Данные и телеметрия остаются на ПК."
            );
        } else {
            print!("{}", self.message);
        }
        io::stdout().flush()
    }

    fn render_agreement(&self, rows: usize, cols: usize) -> io::Result<()> {
        println!(
            "{}{}{}",
            self.theme.status(),
            LICENSE_TEXT[0],
            self.theme.screen()
        );
        for line in LICENSE_TEXT.iter().skip(1).take(rows.saturating_sub(4)) {
            println!("{}", truncate_terminal_line(line, cols));
        }
        print!("Y/Enter — принять; N/Esc — выйти");
        io::stdout().flush()
    }

    fn ensure_cursor_visible(&mut self, viewport_rows: usize) {
        if self.cursor_line < self.scroll_line {
            self.scroll_line = self.cursor_line;
        } else if self.cursor_line >= self.scroll_line + viewport_rows {
            self.scroll_line = self.cursor_line + 1 - viewport_rows;
        }
        self.scroll_line = min(
            self.scroll_line,
            self.lines.len().saturating_sub(viewport_rows),
        );
    }

    fn handle_key(&mut self, key: Key) -> io::Result<bool> {
        self.key_presses = self.key_presses.saturating_add(1);
        if self.key_presses.is_multiple_of(50) {
            self.ad_message = Some(String::from(
                "RustVim Pro: redo, плагины, темы и окна — попробуйте Pro",
            ));
            telemetry::record("pro_ad_shown").ok();
        }
        match self.mode {
            Mode::Agreement => self.handle_agreement(key),
            Mode::Normal => self.handle_normal(key),
            Mode::Insert => {
                self.handle_insert(key);
                Ok(false)
            }
            Mode::VisualLine => {
                self.handle_visual(key);
                Ok(false)
            }
            Mode::Command => self.handle_command(key),
            Mode::Files => self.handle_files(key),
            Mode::Image => {
                self.handle_image(key);
                Ok(false)
            }
            Mode::MarkdownPreview => {
                self.handle_markdown_preview(key);
                Ok(false)
            }
            Mode::Welcome => self.handle_welcome(key),
        }
    }

    fn handle_agreement(&mut self, key: Key) -> io::Result<bool> {
        match key {
            Key::Char('y') | Key::Char('Y') | Key::Enter => {
                license::accept_agreement(&mut self.license_state)?;
                self.agreement_required = false;
                self.mode = Mode::Welcome;
                self.message = String::from("Соглашение принято. Enter или i — открыть буфер.");
                telemetry::record("agreement_accepted").ok();
            }
            Key::Char('n') | Key::Char('N') | Key::Char('q') | Key::Esc | Key::CtrlC => {
                return Ok(true)
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_welcome(&mut self, key: Key) -> io::Result<bool> {
        match key {
            Key::Enter | Key::Char('i') => {
                self.lines = vec![String::new()];
                self.cursor_line = 0;
                self.cursor_col = 0;
                self.mode = Mode::Normal;
                self.message = if self.pro_active {
                    String::from("Empty buffer. Press i to insert; :help for the Vim guide.")
                } else {
                    String::from("Оформите RustVim Pro для AI и премиум-функций.")
                };
            }
            Key::Char('q') | Key::CtrlC => return Ok(true),
            Key::Char(':') => {
                self.command.clear();
                self.command_prompt = ':';
                self.mode = Mode::Command;
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_normal(&mut self, key: Key) -> io::Result<bool> {
        if let Some(pending) = self.pending.take() {
            return self.handle_pending(pending, key);
        }

        match key {
            Key::Char(':') => {
                self.command.clear();
                self.command_prompt = ':';
                self.mode = Mode::Command;
            }
            Key::Char('/') => {
                self.command.clear();
                self.command_prompt = '/';
                self.mode = Mode::Command;
            }
            Key::Char('i') => {
                self.mode = Mode::Insert;
                self.message = String::from("Insert mode.");
            }
            Key::Char('a') => {
                self.move_right();
                self.mode = Mode::Insert;
                self.message = String::from("Append mode.");
            }
            Key::Char('I') => {
                self.cursor_col = first_non_blank(&self.lines[self.cursor_line]);
                self.mode = Mode::Insert;
                self.message = String::from("Insert at first non-blank.");
            }
            Key::Char('A') => {
                self.cursor_col = self.current_line_len();
                self.mode = Mode::Insert;
                self.message = String::from("Append at end of line.");
            }
            Key::Char('o') => {
                self.snapshot();
                self.cursor_line += 1;
                self.lines.insert(self.cursor_line, String::new());
                self.cursor_col = 0;
                self.dirty = true;
                self.mode = Mode::Insert;
            }
            Key::Char('O') => {
                self.snapshot();
                self.lines.insert(self.cursor_line, String::new());
                self.cursor_col = 0;
                self.dirty = true;
                self.mode = Mode::Insert;
            }
            Key::Char('v') | Key::Char('V') => {
                self.mode = Mode::VisualLine;
                self.visual_anchor = self.cursor_line;
                self.message = String::from("Line selection. Use arrows, y, d, Esc.");
            }
            Key::Char('g' | 'd' | 'y' | 'c' | 'r' | '>' | '<') => {
                self.pending = match key {
                    Key::Char(ch) => Some(ch),
                    _ => None,
                }
            }
            Key::Char('p') => self.paste_after(),
            Key::Char('P') => self.paste_before(),
            Key::Char('x') => self.delete_char(),
            Key::Char('u') => self.undo(),
            Key::Char('D') => self.delete_to_end_of_line(),
            Key::Char('C') => self.change_to_end_of_line(),
            Key::Char('J') => self.join_with_next_line(),
            Key::Char('w') => self.move_word_forward(),
            Key::Char('b') => self.move_word_backward(),
            Key::Char('e') => self.move_word_end(),
            Key::Char('n') => self.search_next(),
            Key::Char('N') => self.search_previous(),
            Key::Char('0') => self.cursor_col = 0,
            Key::Char('$') => self.cursor_col = self.current_line_len(),
            Key::Char('G') => self.move_to_last_line(),
            Key::Char(ch @ ('h' | 'j' | 'k' | 'l')) => self.handle_hjkl(ch),
            Key::ArrowUp => self.move_up(),
            Key::ArrowDown => self.move_down(),
            Key::ArrowLeft => self.move_left(),
            Key::ArrowRight => self.move_right(),
            Key::Esc => self.message.clear(),
            Key::CtrlC => return Ok(self.try_exit()),
            _ => {}
        }
        Ok(false)
    }

    fn handle_pending(&mut self, pending: char, key: Key) -> io::Result<bool> {
        match (pending, key) {
            ('g', Key::Char('g')) => {
                self.cursor_line = 0;
                self.clamp_cursor();
            }
            ('d', Key::Char('d')) => self.delete_current_line(),
            ('y', Key::Char('y')) => self.yank_current_line(),
            ('c', Key::Char('c')) => {
                self.delete_current_line();
                self.lines.insert(self.cursor_line, String::new());
                self.mode = Mode::Insert;
                self.message = String::from("Changed line.");
            }
            ('r', Key::Char(ch)) if !ch.is_control() => self.replace_char(ch),
            ('>', Key::Char('>')) => self.indent_current_line(),
            ('<', Key::Char('<')) => self.outdent_current_line(),
            (first, Key::Char(second)) => {
                self.message = format!("Unknown command: {first}{second}")
            }
            (first, _) => self.message = format!("Cancelled pending command: {first}"),
        }
        Ok(false)
    }

    fn handle_insert(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.message = String::from("Normal mode.");
            }
            Key::Enter => {
                self.insert_newline();
            }
            Key::Backspace => self.backspace(),
            Key::Tab => self.ai_complete(),
            Key::ArrowUp => self.move_up(),
            Key::ArrowDown => self.move_down(),
            Key::ArrowLeft => self.move_left(),
            Key::ArrowRight => self.move_right(),
            Key::Char(ch) if !ch.is_control() => self.insert_char(ch),
            _ => {}
        }
    }

    fn handle_visual(&mut self, key: Key) {
        match key {
            Key::Esc | Key::Char('v') | Key::Char('V') => {
                self.mode = Mode::Normal;
                self.message = String::from("Selection cleared.");
            }
            Key::ArrowUp => self.move_up(),
            Key::ArrowDown => self.move_down(),
            Key::Char(ch @ ('h' | 'j' | 'k' | 'l')) => self.handle_hjkl(ch),
            Key::Char('y') => {
                self.yank_selection();
                self.mode = Mode::Normal;
            }
            Key::Char('d') => {
                self.delete_selection();
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn handle_command(&mut self, key: Key) -> io::Result<bool> {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.command.clear();
            }
            Key::Enter => {
                let command = self.command.clone();
                let prompt = self.command_prompt;
                self.command.clear();
                self.mode = Mode::Normal;
                return if prompt == '/' {
                    self.execute_search(&command);
                    Ok(false)
                } else {
                    self.execute_command(&command)
                };
            }
            Key::Backspace => {
                self.command.pop();
            }
            Key::Char(ch) if !ch.is_control() => self.command.push(ch),
            _ => {}
        }
        Ok(false)
    }

    fn execute_command(&mut self, command: &str) -> io::Result<bool> {
        match command {
            "w" => self.save()?,
            "q" => {
                return Ok(self.try_exit());
            }
            "wq" | "x" => {
                self.save()?;
                if self.pro_active {
                    return Ok(true);
                }
                self.message = String::from("Saved, but exit still requires RustVim Pro.");
            }
            "q!" | "qa" | "qa!" => {
                return Ok(self.try_exit());
            }
            "term" => {
                self.open_terminal(None)?;
            }
            "files" | "explore" => self.open_file_manager(None)?,
            "image" => {
                if is_image_path(&self.path) {
                    self.image_path = Some(self.path.clone());
                    self.mode = Mode::Image;
                } else {
                    self.message = String::from("Current file is not a supported image.");
                }
            }
            "preview" | "markdown" | "md-preview" => self.open_markdown_preview(),
            "autocorrect" => self.autocorrect_document(),
            "set autocorrect" => self.set_autocorrect(true)?,
            "set noautocorrect" => self.set_autocorrect(false)?,
            "set render full" => self.set_render_size(None)?,
            other if other.starts_with("set render ") => self.set_render_size(Some(&other[11..]))?,
            "theme" => {
                self.message = format!(
                    "Theme: {}. Pro themes: white, tokyo-night, gruvbox, catppuccin, dracula, nord, one-dark, solarized-dark, rose-pine, monokai, everforest, cyberpunk.",
                    self.theme.name()
                );
            }
            "config path" => {
                self.message = format!("Config: {}", self.config_path.display());
            }
            "config reload" => self.reload_config()?,
            "mcp" | "mcp list" => self.list_mcp_servers(),
            "redo" => self.redo(),
            "tabnew" => self.new_tab(PathBuf::from("untitled.txt")),
            "tabnext" | "tn" => self.switch_tab(1),
            "tabprev" | "tp" => self.switch_tab(usize::MAX),
            "tabs" => self.list_tabs(),
            "plugins" | "plugin list" => self.list_plugins(),
            "battlepass" | "battle-pass" | "bp" => self.message = self.battle_pass.status(),
            "battlepass premium" | "bp premium" if self.access.nitro => {
                self.battle_pass.set_premium(true);
                self.message = String::from("Premium Battle Pass activated through RustVim Nitro.");
            }
            "battlepass premium" | "bp premium" => {
                self.message = String::from("RustVim Nitro is required for the premium Battle Pass.");
            }
            "bp claim" | "battlepass claim" => self.claim_battle_pass(),
            "currency" | "tokens" => self.message = self.economy.status(),
            "slots" | "slot" => self.spin_slots(),
            "lootbox" | "lootbox open" => self.open_lootbox(),
            "nitro" | "subscription nitro" => self.message = if self.access.nitro { String::from("RustVim Nitro active: premium Battle Pass and in-game rewards enabled.") } else { String::from("RustVim Nitro required for the premium Battle Pass.") },
            "bp quest" | "battlepass quest" => self.message = String::from("Quest: edit, save, navigate and use Git to earn XP locally."),
            "git" | "git status" => self.run_git(&["status", "--short"]),
            other if other.starts_with("git ") => {
                let words = parse_command_words(&other[4..]);
                let refs = words.iter().map(String::as_str).collect::<Vec<_>>();
                self.run_git(&refs);
            }
            "help" => {
                self.message = String::from(
                    "i/a/I/A/o/O | arrows | hjkl Pro | V y d | edits | :currency :slots :lootbox :bp premium | :set render",
                );
            }
            "set syntax" if self.pro_active => {
                self.syntax_enabled = true;
                self.config.editor.syntax_highlighting = true;
                self.save_config()?;
                self.message = String::from("Syntax highlighting enabled.");
            }
            "set syntax" => self.show_subscription_prompt(),
            "set nosyntax" if self.pro_active => {
                self.syntax_enabled = false;
                self.config.editor.syntax_highlighting = false;
                self.save_config()?;
                self.message = String::from("Syntax highlighting disabled.");
            }
            "set nosyntax" => self.show_subscription_prompt(),
            other if other == "ai" || other.starts_with("ai ") => {
                self.run_ai(
                    "Answer the user's question about the current file concisely.",
                    other.strip_prefix("ai ").unwrap_or("Summarize this file"),
                    AiAction::Show,
                );
            }
            "ai-summary" => self.run_ai(
                "Summarize the current file, its purpose, structure, and important risks.",
                "Summarize this file.",
                AiAction::Show,
            ),
            "ai-explain" => self.run_ai(
                "Explain the current file clearly for a developer who is new to the codebase.",
                "Explain this file.",
                AiAction::Show,
            ),
            "ai-review" => self.run_ai(
                "Review the current file. Focus on correctness, security, performance, and maintainability. Return actionable findings.",
                "Review this file.",
                AiAction::Show,
            ),
            "ai-docs" => self.run_ai(
                "Generate concise developer documentation for the current file. Return documentation text only.",
                "Document the public behavior, configuration, and important implementation details.",
                AiAction::Show,
            ),
            "ai-optimize" => self.run_ai(
                "Optimize the current file while preserving behavior. Return the complete updated file only, without Markdown fences or explanation.",
                "Improve performance and remove unnecessary work without making the code harder to maintain.",
                AiAction::Replace,
            ),
            "ai-fix" => self.run_ai(
                "Fix bugs and obvious quality problems in the current file. Return the complete updated file only, without Markdown fences or explanation.",
                "Fix this file while preserving its intended behavior.",
                AiAction::Replace,
            ),
            other if other == "ai-refactor" || other.starts_with("ai-refactor ") => self.run_ai(
                "Refactor the current file according to the request. Return the complete updated file only, without Markdown fences or explanation.",
                other.strip_prefix("ai-refactor ").unwrap_or("Improve readability and maintainability."),
                AiAction::Replace,
            ),
            other if other == "ai-tests" || other.starts_with("ai-tests ") => self.run_ai(
                "Generate focused tests for the current file and request. Return code only, without Markdown fences or explanation.",
                other.strip_prefix("ai-tests ").unwrap_or("Generate useful tests."),
                AiAction::Insert,
            ),
            other if other.starts_with("ai-translate ") => self.run_ai(
                "Translate user-facing text and comments in the current file as requested. Preserve code behavior and return the complete updated file only, without Markdown fences or explanation.",
                other.strip_prefix("ai-translate ").unwrap_or_default(),
                AiAction::Replace,
            ),
            other if other == "ai-generate" || other.starts_with("ai-generate ") => self.run_ai(
                "Generate code requested by the user that fits the current file and surrounding style. Return code only, without Markdown fences or explanation.",
                other.strip_prefix("ai-generate ").unwrap_or("Generate a useful implementation for the current context."),
                AiAction::Insert,
            ),
            other if other == "agent" || other.starts_with("agent ") => self.run_ai(
                "Act as an autonomous coding agent. Complete the requested task in the current file, verify consistency mentally, and return the complete updated file only without Markdown fences or explanation.",
                other.strip_prefix("agent ").unwrap_or("Improve this file."),
                AiAction::Replace,
            ),
            "set number" | "set nu" if self.pro_active => {
                self.show_numbers = true;
                self.config.editor.line_numbers = true;
                self.save_config()?;
                self.message = String::from("Line numbers enabled.");
            }
            "set number" | "set nu" => self.show_subscription_prompt(),
            "set nonumber" | "set nonu" if self.pro_active => {
                self.show_numbers = false;
                self.config.editor.line_numbers = false;
                self.save_config()?;
                self.message = String::from("Line numbers disabled.");
            }
            "set nonumber" | "set nonu" => self.show_subscription_prompt(),
            "set altbuffer" | "set alternative-buffer" if self.pro_active => {
                self.set_alternate_buffer(true)?;
                self.config.editor.alternate_buffer = true;
                self.save_config()?;
            }
            "set altbuffer" | "set alternative-buffer" => self.show_subscription_prompt(),
            "set noaltbuffer" | "set noalternative-buffer" if self.pro_active => {
                self.set_alternate_buffer(false)?;
                self.config.editor.alternate_buffer = false;
                self.save_config()?;
            }
            "set noaltbuffer" | "set noalternative-buffer" => self.show_subscription_prompt(),
            digits if digits.chars().all(|ch| ch.is_ascii_digit()) && !digits.is_empty() => {
                let line = digits.parse::<usize>().unwrap_or(1).saturating_sub(1);
                self.cursor_line = min(line, self.lines.len().saturating_sub(1));
                self.clamp_cursor();
            }
            other if other.starts_with("w ") => {
                let path = PathBuf::from(other[2..].trim());
                self.save_as(path)?;
            }
            other if other.starts_with("e ") => {
                let path = PathBuf::from(other[2..].trim());
                self.reload(path)?;
            }
            other if other.starts_with("tabnew ") => self.new_tab(PathBuf::from(other[7..].trim())),
            other if other.starts_with("vsplit ") => self.new_tab(PathBuf::from(other[7..].trim())),
            other if other.starts_with("plugin install ") => self.install_plugin(other[15..].trim())?,
            other if other.starts_with("plugin remove ") => self.remove_plugin(other[14..].trim())?,
            other if other.starts_with("term ") => {
                self.open_terminal(Some(other[5..].trim()))?;
            }
            other if other.starts_with("files ") || other.starts_with("explore ") => {
                let path = other.split_once(' ').map(|(_, path)| path.trim()).unwrap_or(".");
                self.open_file_manager(Some(PathBuf::from(path)))?;
            }
            other if other.starts_with("image ") => {
                let path = PathBuf::from(other[6..].trim());
                if path.is_file() && is_image_path(&path) {
                    self.image_path = Some(path);
                    self.mode = Mode::Image;
                } else {
                    self.message = String::from("Image file not found or unsupported.");
                }
            }
            other if other.starts_with("theme ") => self.set_theme(&other[6..])?,
            other if other.starts_with("mcp add ") => self.add_mcp_server(&other[8..])?,
            other if other.starts_with("mcp remove ") => self.remove_mcp_server(&other[11..])?,
            other if other.starts_with("mcp enable ") => {
                self.set_mcp_server_enabled(&other[11..], true)?
            }
            other if other.starts_with("mcp disable ") => {
                self.set_mcp_server_enabled(&other[12..], false)?
            }
            other if other.starts_with("%s/") || other.starts_with("s/") => self.substitute(other),
            _ => self.message = format!("Unknown command: :{command}"),
        }
        Ok(false)
    }

    fn save_config(&self) -> io::Result<()> {
        config::save(&self.config_path, &self.config)
    }

    fn reload_config(&mut self) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        self.config = config::load(&self.config_path)?;
        self.show_numbers = self.config.editor.line_numbers;
        self.syntax_enabled = self.config.editor.syntax_highlighting;
        self.autocorrect_enabled = self.config.autocorrect.enabled;
        self.theme = if self.pro_active {
            self.config.customization.theme
        } else {
            Theme::White
        };
        let alternate_buffer = self.config.editor.alternate_buffer;
        self.set_alternate_buffer(alternate_buffer)?;
        self.message = format!("Reloaded config: {}", self.config_path.display());
        Ok(())
    }

    fn render_dimensions(&self, terminal_rows: usize, terminal_cols: usize) -> (usize, usize) {
        effective_render_dimensions(
            self.pro_active,
            self.config.editor.render_rows,
            self.config.editor.render_cols,
            terminal_rows,
            terminal_cols,
        )
    }

    fn set_render_size(&mut self, value: Option<&str>) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        let Some(value) = value else {
            self.config.editor.render_rows = None;
            self.config.editor.render_cols = None;
            self.save_config()?;
            self.message = String::from("Render area reset to full terminal size.");
            return Ok(());
        };
        let values = value.split_whitespace().collect::<Vec<_>>();
        let (Some(rows), Some(cols), None) = (values.first(), values.get(1), values.get(2)) else {
            self.message = String::from("Usage: :set render <rows> <cols> or :set render full");
            return Ok(());
        };
        let (Ok(rows), Ok(cols)) = (rows.parse::<usize>(), cols.parse::<usize>()) else {
            self.message = String::from("Render dimensions must be positive integers.");
            return Ok(());
        };
        if rows == 0 || cols == 0 {
            self.message = String::from("Render dimensions must be positive integers.");
            return Ok(());
        }
        self.config.editor.render_rows = Some(rows);
        self.config.editor.render_cols = Some(cols);
        self.save_config()?;
        self.message = format!("Pro render area set to {rows}x{cols}; limited by terminal size.");
        Ok(())
    }

    fn spin_slots(&mut self) {
        self.message = match self.economy.spin() {
            Some(reward) => format!(
                "Слоты: выигрыш {reward} {}. {}",
                economy::CURRENCY_NAME,
                self.economy.status()
            ),
            None => format!(
                "Слоты стоят {} {currency}; доступна только внутриигровая валюта.",
                economy::SPIN_COST,
                currency = economy::CURRENCY_NAME
            ),
        };
    }

    fn open_lootbox(&mut self) {
        self.message = match self.economy.open_lootbox() {
            Some(reward) => format!("{reward}. {}", self.economy.status()),
            None => format!(
                "Лутбокс стоит {} {currency}; доступна только внутриигровая валюта.",
                economy::LOOTBOX_COST,
                currency = economy::CURRENCY_NAME
            ),
        };
    }

    fn set_theme(&mut self, name: &str) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        let Some(theme) = Theme::parse(name) else {
            self.message =
                String::from("Unknown theme. Use :theme to list the available Pro themes.");
            return Ok(());
        };
        self.theme = theme;
        self.config.customization.theme = theme;
        self.save_config()?;
        self.message = format!("Theme changed to {}.", theme.name());
        Ok(())
    }

    fn set_autocorrect(&mut self, enabled: bool) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        self.autocorrect_enabled = enabled;
        self.config.autocorrect.enabled = enabled;
        self.save_config()?;
        self.message = if enabled {
            String::from("Autocorrect enabled.")
        } else {
            String::from("Autocorrect disabled.")
        };
        Ok(())
    }

    fn autocorrect_document(&mut self) {
        if !self.pro_active {
            self.show_subscription_prompt();
            return;
        }
        self.snapshot();
        let mut changes = 0;
        for line in &mut self.lines {
            let (corrected, line_changes) =
                autocorrect_text(line, &self.config.autocorrect.replacements);
            if line_changes > 0 {
                *line = corrected;
                changes += line_changes;
            }
        }
        if changes == 0 {
            let _ = self.undo.pop();
            self.message = String::from("Autocorrect found no replacements.");
        } else {
            self.dirty = true;
            self.clamp_cursor();
            self.message = format!("Autocorrect applied {changes} replacement(s).");
        }
    }

    fn autocorrect_word_before_cursor(&mut self) {
        let line = &mut self.lines[self.cursor_line];
        let before = &line[..self.cursor_col];
        let start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| !is_word_char(*ch))
            .map(|(index, ch)| index + ch.len_utf8())
            .unwrap_or(0);
        let word = &line[start..self.cursor_col];
        let Some(replacement) = self.config.autocorrect.replacements.get(word) else {
            return;
        };
        let replacement = replacement.clone();
        line.replace_range(start..self.cursor_col, &replacement);
        self.cursor_col = start + replacement.len();
    }

    fn open_markdown_preview(&mut self) {
        if !self.pro_active {
            self.show_subscription_prompt();
        } else if !matches!(syntax_for_path(&self.path), Some(Syntax::Markdown)) {
            self.message = String::from("Markdown preview requires a .md or .markdown file.");
        } else {
            self.mode = Mode::MarkdownPreview;
        }
    }

    fn render_markdown_preview(&self, rows: usize, cols: usize) -> io::Result<()> {
        let width = cols.saturating_sub(2).max(1);
        let rendered = render_markdown(&self.lines, width);
        println!(
            "{} MARKDOWN PREVIEW  {} {}\r",
            self.theme.status(),
            self.path.display(),
            self.theme.screen()
        );
        for line in rendered.into_iter().take(rows.saturating_sub(3)) {
            println!("{}\r", truncate_terminal_line(&line, cols));
        }
        print!("Esc: editor  r: refresh");
        io::stdout().flush()
    }

    fn render_welcome(&self, rows: usize, cols: usize) -> io::Result<()> {
        let content_width = cols.saturating_sub(4).max(24);
        let mut content = vec![
            String::from("RustVim"),
            String::from("Vim guide / стартовый экран"),
            String::new(),
        ];
        if self.pro_active {
            content.push(String::from("RustVim Pro активен."));
        } else {
            content.push(String::from(
                "Оформите подписку RustVim Pro для AI и премиум-функций.",
            ));
            content.push(String::from(
                "Без подписки доступен только стандартный белый интерфейс.",
            ));
        }
        content.extend([
            String::new(),
            String::from("Быстрый гайд по Vim:"),
            String::from("  i / a       начать ввод до / после курсора"),
            String::from("  Esc         вернуться в normal mode"),
            String::from("  h j k l     перемещение (hjkl доступно в Pro)"),
            String::from("  w / b       следующее / предыдущее слово"),
            String::from("  dd / yy     удалить / скопировать строку"),
            String::from("  p / P        вставить после / перед курсором"),
            String::from("  u            отменить последнее изменение"),
            String::from("  /текст       поиск по файлу"),
            String::from("  :w           сохранить файл"),
            String::from("  :q           выйти (доступно в Pro)"),
            String::new(),
            String::from("Enter или i — открыть пустой буфер; :help — справка; q — выход"),
        ]);
        println!(
            "{} RustVim — Welcome {}",
            self.theme.status(),
            self.theme.screen()
        );
        for line in content.into_iter().take(rows.saturating_sub(3)) {
            println!("  {}\r", truncate_terminal_line(&line, content_width));
        }
        print!("{}", self.message);
        io::stdout().flush()
    }

    fn handle_markdown_preview(&mut self, key: Key) {
        if matches!(key, Key::Esc | Key::Backspace | Key::Char('q')) {
            self.mode = Mode::Normal;
            self.message = String::from("Markdown preview closed.");
        }
    }

    fn list_mcp_servers(&mut self) {
        if !self.pro_active {
            self.show_subscription_prompt();
            return;
        }
        if self.config.mcp_servers.is_empty() {
            self.message = String::from("No MCP servers. Use :mcp add <name> <command> [args...].");
            return;
        }
        self.message = format!(
            "MCP: {}",
            self.config
                .mcp_servers
                .iter()
                .map(|(name, server)| format!(
                    "{}{}",
                    name,
                    if server.enabled { "" } else { " (disabled)" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    fn add_mcp_server(&mut self, spec: &str) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        let parts = parse_command_words(spec);
        if parts.len() < 2 {
            self.message = String::from("Usage: :mcp add <name> <command> [args...]");
            return Ok(());
        }
        let name = parts[0].clone();
        self.config.mcp_servers.insert(
            name.clone(),
            McpServerConfig {
                command: parts[1].clone(),
                args: parts[2..].to_vec(),
                cwd: None,
                enabled: true,
            },
        );
        self.save_config()?;
        self.message = format!(
            "MCP server '{name}' added to {}.",
            self.config_path.display()
        );
        Ok(())
    }

    fn remove_mcp_server(&mut self, name: &str) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        let name = name.trim();
        if self.config.mcp_servers.remove(name).is_some() {
            self.save_config()?;
            self.message = format!("MCP server '{name}' removed.");
        } else {
            self.message = format!("MCP server '{name}' not found.");
        }
        Ok(())
    }

    fn set_mcp_server_enabled(&mut self, name: &str, enabled: bool) -> io::Result<()> {
        if !self.pro_active {
            self.show_subscription_prompt();
            return Ok(());
        }
        let name = name.trim();
        let Some(server) = self.config.mcp_servers.get_mut(name) else {
            self.message = format!("MCP server '{name}' not found.");
            return Ok(());
        };
        server.enabled = enabled;
        self.save_config()?;
        self.message = format!(
            "MCP server '{name}' {}.",
            if enabled { "enabled" } else { "disabled" }
        );
        Ok(())
    }

    fn empty_line_marker(&self) -> &str {
        if self.pro_active {
            &self.config.customization.empty_line_marker
        } else {
            "~"
        }
    }

    fn line_marker(&self, cursor: bool, selected: bool) -> String {
        if cursor && self.battle_pass.frontier_cursor_unlocked() {
            return if selected {
                String::from("◆*")
            } else {
                String::from("◆ ")
            };
        }
        if !self.pro_active {
            return line_marker(cursor, selected).to_owned();
        }
        match (cursor, selected) {
            (true, true) => format!("{}*", self.config.customization.cursor_marker.trim_end()),
            (true, false) => self.config.customization.cursor_marker.clone(),
            (false, true) => self.config.customization.selected_marker.clone(),
            (false, false) => String::from("  "),
        }
    }

    fn set_alternate_buffer(&mut self, enabled: bool) -> io::Result<()> {
        if self.use_alternate_buffer == enabled {
            self.message = if enabled {
                String::from("Alternative buffer is already enabled.")
            } else {
                String::from("Alternative buffer is already disabled.")
            };
            return Ok(());
        }

        self.use_alternate_buffer = enabled;
        if enabled {
            print!("\x1b[?1049h\x1b[2J\x1b[H");
            self.message = String::from("Alternative buffer enabled.");
        } else {
            print!("\x1b[?1049l");
            self.message = String::from("Alternative buffer disabled.");
        }
        io::stdout().flush()
    }

    fn save(&mut self) -> io::Result<()> {
        fs::write(&self.path, serialize_editor_lines(&self.lines))?;
        self.write_free_metadata(&self.path)?;
        self.dirty = false;
        self.message = format!("Saved: {}", self.path.display());
        Ok(())
    }

    fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        if path.as_os_str().is_empty() {
            self.message = String::from("Missing file name.");
            return Ok(());
        }
        fs::write(&path, serialize_editor_lines(&self.lines))?;
        self.write_free_metadata(&path)?;
        self.path = path;
        self.dirty = false;
        self.message = format!("Saved as: {}", self.path.display());
        Ok(())
    }

    fn write_free_metadata(&self, path: &Path) -> io::Result<()> {
        if self.pro_active {
            return Ok(());
        }
        let metadata = path.with_extension(format!(
            "{}rustvim-meta.toml",
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("{e}."))
                .unwrap_or_default()
        ));
        fs::write(metadata, "edited_with = \"rustvim-free\"\nwatermark = \"RustVim Free\"\ngit_integration = \"rustvim-free\"\n")
    }

    fn run_git(&mut self, args: &[&str]) {
        let cwd = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut git_args = args.to_vec();
        if !self.pro_active && args.first() == Some(&"commit") {
            git_args.extend(["--trailer", "RustVim-Free=true"]);
        }
        match Command::new("git")
            .args(["-C", cwd.to_string_lossy().as_ref()])
            .args(&git_args)
            .output()
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(if output.stdout.is_empty() {
                    &output.stderr
                } else {
                    &output.stdout
                });
                self.message = format!("git: {}", truncate_message(text.trim(), 160));
                telemetry::record("git_command").ok();
            }
            Err(error) => self.message = format!("git error: {error}"),
        }
    }

    fn new_tab(&mut self, path: PathBuf) {
        self.tabs.push(TabState {
            path: self.path.clone(),
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
            dirty: self.dirty,
        });
        self.current_tab = self.tabs.len();
        self.path = path;
        self.lines = fs::read_to_string(&self.path)
            .map(|text| split_editor_lines(&text))
            .unwrap_or_else(|_| vec![String::new()]);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.dirty = false;
        self.message = format!("Opened tab {}.", self.path.display());
        telemetry::record("tab_opened").ok();
    }

    fn switch_tab(&mut self, direction: usize) {
        if self.tabs.is_empty() {
            self.message = String::from("No other tabs. Use :tabnew path.");
            return;
        }
        let current = TabState {
            path: self.path.clone(),
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
            dirty: self.dirty,
        };
        let total = self.tabs.len() + 1;
        let selected = if direction == usize::MAX {
            let selected = self.tabs.pop().expect("tab list is not empty");
            self.tabs.insert(0, current);
            self.current_tab = (self.current_tab + total - 1) % total;
            selected
        } else {
            let selected = self.tabs.remove(0);
            self.tabs.push(current);
            self.current_tab = (self.current_tab + 1) % total;
            selected
        };
        self.path = selected.path;
        self.lines = selected.lines;
        self.cursor_line = selected.cursor_line;
        self.cursor_col = selected.cursor_col;
        self.dirty = selected.dirty;
        self.clamp_cursor();
        self.message = format!(
            "Tab {}/{}: {}",
            self.current_tab + 1,
            self.tabs.len() + 1,
            self.path.display()
        );
    }

    fn list_tabs(&mut self) {
        self.message = format!(
            "Tab {}/{}: {} (use :tabnew, :tabnext, :tabprev)",
            self.current_tab + 1,
            self.tabs.len() + 1,
            self.path.display()
        );
    }

    fn list_plugins(&mut self) {
        if !self.access.plugins {
            self.message = format!(
                "Plugins require Pro; trial expired ({} hours left).",
                self.access.plugins_trial_remaining.as_secs() / 3600
            );
            return;
        }
        self.message = match self.plugins.list() {
            Ok(names) if names.is_empty() => String::from("No plugins installed."),
            Ok(names) => format!("Plugins: {}", names.join(", ")),
            Err(error) => format!("Plugin error: {error}"),
        };
    }

    fn claim_battle_pass(&mut self) {
        match self.battle_pass.claim() {
            Some(reward) => {
                self.message = reward.to_owned();
                telemetry::record("battle_pass_reward_claimed").ok();
            }
            None => self.message = String::from("No Battle Pass reward available yet."),
        }
    }

    fn install_plugin(&mut self, name: &str) -> io::Result<()> {
        if !self.access.plugins {
            license::start_plugins_trial(&mut self.license_state, SystemTime::now())?;
            self.access = license::access(
                &self.license_state,
                nitro_active_from_env(),
                pro_active_from_env(),
                SystemTime::now(),
            );
        }
        if !self.access.plugins {
            self.message = String::from("Plugin trial expired. RustVim Pro is required.");
            return Ok(());
        }
        self.plugins.install(name)?;
        self.message = format!("Plugin installed: {name}");
        telemetry::record("plugin_installed").ok();
        Ok(())
    }

    fn remove_plugin(&mut self, name: &str) -> io::Result<()> {
        self.plugins.remove(name)?;
        self.message = format!("Plugin removed: {name}");
        Ok(())
    }

    fn reload(&mut self, path: PathBuf) -> io::Result<()> {
        if path.as_os_str().is_empty() {
            self.message = String::from("Missing file name.");
            return Ok(());
        }
        let content = fs::read_to_string(&path)?;
        let mut lines = split_editor_lines(&content);
        if lines.is_empty() {
            lines.push(String::new());
        }
        self.snapshot();
        self.path = path;
        self.lines = lines;
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_line = 0;
        self.dirty = false;
        self.message = String::from("File loaded.");
        Ok(())
    }

    fn execute_search(&mut self, query: &str) {
        if query.is_empty() {
            self.message = String::from("Empty search.");
            return;
        }
        self.search = Some(query.to_owned());
        self.search_next();
    }

    fn substitute(&mut self, command: &str) {
        let global = command.ends_with("/g");
        let trimmed = if global {
            &command[..command.len().saturating_sub(2)]
        } else {
            command
        };
        let spec = trimmed
            .strip_prefix("%s/")
            .or_else(|| trimmed.strip_prefix("s/"));
        let Some(spec) = spec else {
            self.message = String::from("Substitute syntax: :%s/old/new/g");
            return;
        };
        let mut parts = spec.splitn(3, '/');
        let old = parts.next().unwrap_or("");
        let new = parts.next().unwrap_or("");
        if old.is_empty() {
            self.message = String::from("Substitute needs a non-empty pattern.");
            return;
        }

        self.snapshot();
        let mut changed = 0;
        if command.starts_with("%s/") {
            for line in &mut self.lines {
                if line.contains(old) {
                    changed += line.matches(old).count();
                    *line = if global {
                        line.replace(old, new)
                    } else {
                        line.replacen(old, new, 1)
                    };
                }
            }
        } else if self.lines[self.cursor_line].contains(old) {
            changed = self.lines[self.cursor_line].matches(old).count();
            self.lines[self.cursor_line] = if global {
                self.lines[self.cursor_line].replace(old, new)
            } else {
                changed = 1;
                self.lines[self.cursor_line].replacen(old, new, 1)
            };
        }

        if changed == 0 {
            let _ = self.undo.pop();
            self.message = format!("Pattern not found: {old}");
        } else {
            self.dirty = true;
            self.clamp_cursor();
            self.message = format!("Substituted {changed} occurrence(s).");
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.snapshot();
        if self.autocorrect_enabled && (ch.is_whitespace() || is_autocorrect_separator(ch)) {
            self.autocorrect_word_before_cursor();
        }
        self.lines[self.cursor_line].insert(self.cursor_col, ch);
        self.cursor_col += ch.len_utf8();
        self.dirty = true;
    }

    fn insert_newline(&mut self) {
        self.snapshot();
        self.clamp_cursor();
        let rest = self.lines[self.cursor_line].split_off(self.cursor_col);
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, rest);
        self.cursor_col = 0;
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.snapshot();
            let line = &mut self.lines[self.cursor_line];
            if let Some((index, _)) = line[..self.cursor_col].char_indices().last() {
                line.drain(index..self.cursor_col);
                self.cursor_col = index;
                self.dirty = true;
            }
        } else if self.cursor_line > 0 {
            self.snapshot();
            let removed = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&removed);
            self.dirty = true;
        }
    }

    fn delete_char(&mut self) {
        if self.lines[self.cursor_line].is_empty() {
            return;
        }
        self.snapshot();
        let line = &mut self.lines[self.cursor_line];
        if self.cursor_col >= line.len() {
            self.cursor_col = line.len().saturating_sub(1);
        }
        if let Some(ch) = line[self.cursor_col..].chars().next() {
            line.drain(self.cursor_col..self.cursor_col + ch.len_utf8());
            self.dirty = true;
            self.message = String::from("Deleted character.");
        }
    }

    fn replace_char(&mut self, ch: char) {
        if self.lines[self.cursor_line].is_empty() {
            self.message = String::from("Nothing to replace.");
            return;
        }
        self.snapshot();
        let line = &mut self.lines[self.cursor_line];
        if self.cursor_col >= line.len() {
            self.cursor_col = line.len().saturating_sub(1);
        }
        if let Some(old) = line[self.cursor_col..].chars().next() {
            line.drain(self.cursor_col..self.cursor_col + old.len_utf8());
            line.insert(self.cursor_col, ch);
            self.dirty = true;
            self.message = String::from("Replaced character.");
        }
    }

    fn delete_to_end_of_line(&mut self) {
        self.snapshot();
        self.lines[self.cursor_line].truncate(self.cursor_col);
        self.dirty = true;
        self.message = String::from("Deleted to end of line.");
    }

    fn change_to_end_of_line(&mut self) {
        self.delete_to_end_of_line();
        self.mode = Mode::Insert;
        self.message = String::from("Change to end of line.");
    }

    fn delete_current_line(&mut self) {
        self.snapshot();
        self.clipboard = vec![self.lines.remove(self.cursor_line)];
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = min(self.cursor_line, self.lines.len() - 1);
        self.cursor_col = 0;
        self.dirty = true;
        self.message = String::from("Deleted line.");
    }

    fn yank_current_line(&mut self) {
        self.clipboard = vec![self.lines[self.cursor_line].clone()];
        self.message = String::from("Yanked line.");
    }

    fn join_with_next_line(&mut self) {
        if self.cursor_line + 1 >= self.lines.len() {
            self.message = String::from("No next line to join.");
            return;
        }
        self.snapshot();
        let next = self.lines.remove(self.cursor_line + 1);
        if !self.lines[self.cursor_line].ends_with(' ') && !next.is_empty() {
            self.lines[self.cursor_line].push(' ');
        }
        self.lines[self.cursor_line].push_str(next.trim_start());
        self.dirty = true;
        self.message = String::from("Joined lines.");
    }

    fn indent_current_line(&mut self) {
        self.snapshot();
        self.lines[self.cursor_line].insert_str(0, "    ");
        self.cursor_col += 4;
        self.dirty = true;
        self.message = String::from("Indented line.");
    }

    fn outdent_current_line(&mut self) {
        let remove = self.lines[self.cursor_line]
            .chars()
            .take_while(|ch| *ch == ' ')
            .take(4)
            .count();
        if remove == 0 {
            self.message = String::from("Line is not indented.");
            return;
        }
        self.snapshot();
        self.lines[self.cursor_line].drain(0..remove);
        self.cursor_col = self.cursor_col.saturating_sub(remove);
        self.dirty = true;
        self.message = String::from("Outdented line.");
    }

    fn yank_selection(&mut self) {
        let (start, end) = self.selection_bounds();
        self.clipboard = self.lines[start..=end].to_vec();
        self.message = format!("Yanked {} line(s).", self.clipboard.len());
    }

    fn delete_selection(&mut self) {
        let (start, end) = self.selection_bounds();
        self.snapshot();
        self.clipboard = self.lines[start..=end].to_vec();
        self.lines.drain(start..=end);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = min(start, self.lines.len() - 1);
        self.cursor_col = 0;
        self.dirty = true;
        self.message = format!("Deleted {} line(s).", self.clipboard.len());
    }

    fn paste_after(&mut self) {
        if self.clipboard.is_empty() {
            self.message = String::from("Clipboard is empty.");
            return;
        }
        self.snapshot();
        let insert_at = min(self.cursor_line + 1, self.lines.len());
        for (offset, line) in self.clipboard.clone().into_iter().enumerate() {
            self.lines.insert(insert_at + offset, line);
        }
        self.cursor_line = insert_at;
        self.cursor_col = 0;
        self.dirty = true;
        self.message = String::from("Pasted after cursor.");
    }

    fn paste_before(&mut self) {
        if self.clipboard.is_empty() {
            self.message = String::from("Clipboard is empty.");
            return;
        }
        self.snapshot();
        let insert_at = self.cursor_line;
        for (offset, line) in self.clipboard.clone().into_iter().enumerate() {
            self.lines.insert(insert_at + offset, line);
        }
        self.cursor_col = 0;
        self.dirty = true;
        self.message = String::from("Pasted before cursor.");
    }

    fn undo(&mut self) {
        if let Some(lines) = self.undo.pop() {
            self.redo.push(self.lines.clone());
            self.lines = lines;
            self.cursor_line = min(self.cursor_line, self.lines.len() - 1);
            self.clamp_cursor();
            self.dirty = true;
            self.message = String::from("Undo.");
        } else {
            self.message = String::from("Nothing to undo.");
        }
    }

    fn redo(&mut self) {
        if !self.pro_active {
            self.show_subscription_prompt();
            return;
        }
        if let Some(lines) = self.redo.pop() {
            self.undo.push(self.lines.clone());
            self.lines = lines;
            self.clamp_cursor();
            self.dirty = true;
            self.message = String::from("Redo.");
        } else {
            self.message = String::from("Nothing to redo.");
        }
    }

    fn snapshot(&mut self) {
        self.undo.push(self.lines.clone());
        self.redo.clear();
        self.battle_pass.add_xp(1).ok();
        self.economy.earn(1).ok();
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    fn selection_bounds(&self) -> (usize, usize) {
        (
            min(self.visual_anchor, self.cursor_line),
            max(self.visual_anchor, self.cursor_line),
        )
    }

    fn move_up(&mut self) {
        self.cursor_line = self.cursor_line.saturating_sub(1);
        self.clamp_cursor();
    }

    fn move_down(&mut self) {
        self.cursor_line = min(self.cursor_line + 1, self.lines.len() - 1);
        self.clamp_cursor();
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            if let Some((index, _)) = self.lines[self.cursor_line][..self.cursor_col]
                .char_indices()
                .last()
            {
                self.cursor_col = index;
            }
        }
    }

    fn move_right(&mut self) {
        let line = &self.lines[self.cursor_line];
        if self.cursor_col < line.len() {
            if let Some(ch) = line[self.cursor_col..].chars().next() {
                self.cursor_col += ch.len_utf8();
            }
        }
    }

    fn move_to_last_line(&mut self) {
        self.cursor_line = self.lines.len() - 1;
        self.clamp_cursor();
    }

    fn move_word_forward(&mut self) {
        let mut pos = self.absolute_cursor();
        let text = self.full_text();
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        while let Some((byte, ch)) = next_char_at(&chars, pos) {
            if *byte > pos && is_word_char(*ch) {
                pos = *byte;
                break;
            }
            pos = byte + ch.len_utf8();
        }
        self.set_absolute_cursor(pos);
    }

    fn move_word_backward(&mut self) {
        let text = self.full_text();
        let before = &text[..min(self.absolute_cursor(), text.len())];
        let mut target = None;
        let mut in_word = false;
        for (index, ch) in before.char_indices().rev() {
            if is_word_char(ch) {
                target = Some(index);
                in_word = true;
            } else if in_word {
                break;
            }
        }
        if let Some(pos) = target {
            self.set_absolute_cursor(pos);
        }
    }

    fn move_word_end(&mut self) {
        let mut pos = self.absolute_cursor();
        let text = self.full_text();
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut seen_word = false;
        while let Some((byte, ch)) = next_char_at(&chars, pos) {
            if is_word_char(*ch) {
                seen_word = true;
                pos = *byte + ch.len_utf8();
            } else if seen_word {
                break;
            } else {
                pos = *byte + ch.len_utf8();
            }
        }
        self.set_absolute_cursor(pos.saturating_sub(1));
    }

    fn search_next(&mut self) {
        let Some(query) = self.search.clone() else {
            self.message = String::from("No previous search.");
            return;
        };
        let text = self.full_text();
        let start = min(self.absolute_cursor() + 1, text.len());
        let found = text[start..]
            .find(&query)
            .map(|offset| start + offset)
            .or_else(|| text[..start].find(&query));
        if let Some(pos) = found {
            self.set_absolute_cursor(pos);
            self.message = format!("Found: {query}");
        } else {
            self.message = format!("Pattern not found: {query}");
        }
    }

    fn search_previous(&mut self) {
        let Some(query) = self.search.clone() else {
            self.message = String::from("No previous search.");
            return;
        };
        let text = self.full_text();
        let start = min(self.absolute_cursor(), text.len());
        let found = text[..start]
            .rfind(&query)
            .or_else(|| text[start..].rfind(&query).map(|offset| start + offset));
        if let Some(pos) = found {
            self.set_absolute_cursor(pos);
            self.message = format!("Found: {query}");
        } else {
            self.message = format!("Pattern not found: {query}");
        }
    }

    fn absolute_cursor(&self) -> usize {
        let mut pos = 0;
        for index in 0..self.cursor_line {
            pos += self.lines[index].len() + 1;
        }
        pos + self.cursor_col
    }

    fn set_absolute_cursor(&mut self, mut pos: usize) {
        for (index, line) in self.lines.iter().enumerate() {
            if pos <= line.len() {
                self.cursor_line = index;
                self.cursor_col = pos;
                self.clamp_cursor();
                return;
            }
            pos = pos.saturating_sub(line.len() + 1);
        }
        self.move_to_last_line();
    }

    fn full_text(&self) -> String {
        self.lines.join("\n")
    }

    fn open_terminal(&mut self, command: Option<&str>) -> io::Result<()> {
        RawTerminal::suspend_for_child()?;

        let status = if let Some(command) = command.filter(|command| !command.is_empty()) {
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
        } else {
            let shell = env::var("SHELL").unwrap_or_else(|_| String::from("sh"));
            Command::new(shell)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
        };

        RawTerminal::resume_from_child(self.use_alternate_buffer)?;
        match status {
            Ok(status) => self.message = format!("Terminal exited with status {status}."),
            Err(error) => self.message = format!("Terminal failed: {error}"),
        }
        Ok(())
    }

    fn open_file_manager(&mut self, path: Option<PathBuf>) -> io::Result<()> {
        if let Some(path) = path {
            self.browser_dir = if path.is_dir() {
                path
            } else {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            };
        }
        self.refresh_file_manager()?;
        self.mode = Mode::Files;
        self.message = String::from("File manager: arrows/j/k, Enter, Backspace, Esc.");
        Ok(())
    }

    fn refresh_file_manager(&mut self) -> io::Result<()> {
        let mut entries = fs::read_dir(&self.browser_dir)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .is_dir()
                .cmp(&left.is_dir())
                .then_with(|| left.file_name().cmp(&right.file_name()))
        });
        self.browser_entries = entries;
        self.browser_selected = min(
            self.browser_selected,
            self.browser_entries.len().saturating_sub(1),
        );
        Ok(())
    }

    fn render_file_manager(&self, rows: usize, cols: usize) -> io::Result<()> {
        println!(
            "{} FILES  {} {}\r",
            self.theme.status(),
            self.browser_dir.display(),
            self.theme.screen()
        );
        let viewport_rows = rows.saturating_sub(3).max(1);
        let start = self
            .browser_selected
            .saturating_sub(viewport_rows.saturating_sub(1));
        let end = min(start + viewport_rows, self.browser_entries.len());
        for index in start..end {
            let path = &self.browser_entries[index];
            let marker = if index == self.browser_selected {
                ">"
            } else {
                " "
            };
            let kind = if path.is_dir() {
                "📁"
            } else if is_image_path(path) {
                "🖼"
            } else {
                " "
            };
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            println!(
                "{} {} {}\r",
                marker,
                kind,
                truncate_terminal_line(&name, cols.saturating_sub(4))
            );
        }
        for _ in end.saturating_sub(start)..viewport_rows {
            println!(
                "{}{}\r",
                self.theme.screen(),
                truncate_terminal_line(self.empty_line_marker(), cols)
            );
        }
        print!("Enter: open  Backspace: parent  r: refresh  Esc: editor");
        io::stdout().flush()
    }

    fn handle_files(&mut self, key: Key) -> io::Result<bool> {
        match key {
            Key::Esc => self.mode = Mode::Normal,
            Key::ArrowUp | Key::Char('k') => {
                self.browser_selected = self.browser_selected.saturating_sub(1)
            }
            Key::ArrowDown | Key::Char('j') => {
                self.browser_selected = min(
                    self.browser_selected + 1,
                    self.browser_entries.len().saturating_sub(1),
                )
            }
            Key::Backspace | Key::Char('h') => {
                if let Some(parent) = self.browser_dir.parent() {
                    self.browser_dir = parent.to_path_buf();
                    self.browser_selected = 0;
                    self.refresh_file_manager()?;
                }
            }
            Key::Char('r') => self.refresh_file_manager()?,
            Key::Enter | Key::Char('l') => self.open_selected_entry()?,
            _ => {}
        }
        Ok(false)
    }

    fn open_selected_entry(&mut self) -> io::Result<()> {
        let Some(path) = self.browser_entries.get(self.browser_selected).cloned() else {
            return Ok(());
        };
        if path.is_dir() {
            self.browser_dir = path;
            self.browser_selected = 0;
            self.refresh_file_manager()?;
        } else if is_image_path(&path) {
            self.image_path = Some(path);
            self.mode = Mode::Image;
        } else {
            self.reload(path)?;
            self.mode = Mode::Normal;
        }
        Ok(())
    }

    fn render_image_preview(&self, rows: usize, cols: usize) -> io::Result<()> {
        let Some(path) = self.image_path.as_deref() else {
            return Ok(());
        };
        println!("\x1b[7m IMAGE  {} \x1b[0m\r", path.display());
        print!("{}", render_image(path, cols, rows.saturating_sub(3))?);
        print!("\r\nEsc: back  o: open externally");
        io::stdout().flush()
    }

    fn handle_image(&mut self, key: Key) {
        match key {
            Key::Esc | Key::Backspace => self.mode = Mode::Files,
            Key::Char('o') => {
                if let Some(path) = self.image_path.as_deref() {
                    self.message = match open_external(path) {
                        Ok(()) => String::from("Opened image externally."),
                        Err(error) => format!("Failed to open image: {error}"),
                    };
                }
            }
            _ => {}
        }
    }

    fn clamp_cursor(&mut self) {
        self.cursor_col = min(self.cursor_col, self.current_line_len());
    }

    fn current_line_len(&self) -> usize {
        self.lines[self.cursor_line].len()
    }

    fn syntax_active(&self) -> bool {
        self.pro_active && self.syntax_enabled && syntax_for_path(&self.path).is_some()
    }

    fn run_ai(&mut self, instructions: &str, prompt: &str, action: AiAction) {
        if !self.pro_active {
            self.message = SUBSCRIPTION_PROMPT.to_owned();
            return;
        }
        self.message = String::from("AI request in progress...");
        let input = self.ai_context(prompt);
        let result = std::panic::catch_unwind(|| {
            AiClient::from_env().and_then(|client| client.respond(instructions, &input))
        });
        match result {
            Ok(Ok(response)) => self.apply_ai_response(action, &response),
            Ok(Err(error)) => {
                self.message = format!("AI error: {}", truncate_message(&error.to_string(), 160))
            }
            Err(_) => self.message = String::from("AI provider crashed; editor recovered safely."),
        }
    }

    fn ai_context(&self, prompt: &str) -> String {
        const MAX_CONTEXT_BYTES: usize = 100_000;
        let text = self.full_text();
        if text.len() <= MAX_CONTEXT_BYTES {
            return format!(
                "File: {}\nUser request: {prompt}\n\nCurrent file:\n{text}",
                self.path.display()
            );
        }

        let cursor = self.absolute_cursor();
        let half = MAX_CONTEXT_BYTES / 2;
        let start = floor_char_boundary(&text, cursor.saturating_sub(half));
        let end = ceil_char_boundary(&text, min(text.len(), start + MAX_CONTEXT_BYTES));
        format!(
            "File: {}\nUser request: {prompt}\nThe file is large; this excerpt surrounds the cursor at byte {cursor} of {}.\n\n{}",
            self.path.display(),
            text.len(),
            &text[start..end]
        )
    }

    fn apply_ai_response(&mut self, action: AiAction, response: &str) {
        let response = strip_code_fence(response).trim();
        match action {
            AiAction::Show => {
                self.message = format!("AI: {}", truncate_message(response, 180));
            }
            AiAction::Insert => self.insert_ai_text(response),
            AiAction::Replace => {
                self.snapshot();
                self.lines = split_editor_lines(response);
                if self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                self.cursor_line = 0;
                self.cursor_col = 0;
                self.dirty = true;
                self.message = String::from("AI updated the current file.");
            }
        }
    }

    fn insert_ai_text(&mut self, text: &str) {
        self.snapshot();
        let position = self.absolute_cursor();
        let mut content = self.full_text();
        content.insert_str(position, text);
        self.lines = split_editor_lines(&content);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.set_absolute_cursor(position + text.len());
        self.dirty = true;
        self.message = String::from("AI content inserted.");
    }

    fn ai_complete(&mut self) {
        if !self.pro_active {
            self.message = SUBSCRIPTION_PROMPT.to_owned();
            return;
        }
        let position = self.absolute_cursor();
        let text = self.full_text();
        let prefix_start = floor_char_boundary(&text, position.saturating_sub(4000));
        let suffix_end = ceil_char_boundary(&text, min(text.len(), position + 1000));
        let input = format!(
            "Complete code at <CURSOR>. Return only the text to insert.\n\n{}<CURSOR>{}",
            &text[prefix_start..position],
            &text[position..suffix_end]
        );
        match AiClient::from_env().and_then(|client| {
            client.respond(
                "Provide a short, context-aware code completion. Return only the missing text, without Markdown or explanation.",
                &input,
            )
        }) {
            Ok(response) => self.insert_ai_text(strip_code_fence(&response).trim()),
            Err(error) => self.message = format!("AI completion error: {}", truncate_message(&error.to_string(), 140)),
        }
    }

    fn handle_hjkl(&mut self, key: char) {
        if !self.pro_active {
            self.show_subscription_prompt();
            return;
        }

        match key {
            'h' => self.move_left(),
            'j' => self.move_down(),
            'k' => self.move_up(),
            'l' => self.move_right(),
            _ => unreachable!(),
        }
    }

    fn try_exit(&mut self) -> bool {
        if self.pro_active {
            return true;
        }
        self.show_subscription_prompt();
        false
    }

    fn show_subscription_prompt(&mut self) {
        self.message = SUBSCRIPTION_PROMPT.to_owned();
    }
}

fn pro_active_from_env() -> bool {
    env::var(PRO_ENV_VAR)
        .map(|value| env_value_is_enabled(&value))
        .unwrap_or(false)
}

fn nitro_active_from_env() -> bool {
    env::var(NITRO_ENV_VAR)
        .map(|value| env_value_is_enabled(&value))
        .unwrap_or(false)
        || pro_active_from_env()
}

fn env_value_is_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "enabled"
    )
}

#[derive(Clone, Copy)]
enum Syntax {
    Rust,
    Python,
    JavaScript,
    Json,
    Shell,
    Markdown,
}

fn syntax_for_path(path: &Path) -> Option<Syntax> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "rs" => Some(Syntax::Rust),
        "py" => Some(Syntax::Python),
        "js" | "jsx" | "ts" | "tsx" => Some(Syntax::JavaScript),
        "json" => Some(Syntax::Json),
        "sh" | "bash" | "zsh" => Some(Syntax::Shell),
        "md" | "markdown" => Some(Syntax::Markdown),
        _ => None,
    }
}

fn highlight_syntax(line: &str, syntax: Option<Syntax>, theme: Theme) -> String {
    let Some(syntax) = syntax else {
        return line.to_owned();
    };
    let keywords = match syntax {
        Syntax::Rust => "fn let mut struct enum impl pub use mod match if else for in return Self self true false",
        Syntax::Python => "def class import from as if elif else for while in return True False None and or not",
        Syntax::JavaScript => "const let var function return if else for while import from export true false null undefined",
        Syntax::Json => "true false null",
        Syntax::Shell => "if then else fi for in do done case esac function export local",
        Syntax::Markdown => "",
    };
    let comment_markers = match syntax {
        Syntax::Rust | Syntax::JavaScript => &["//"][..],
        Syntax::Python | Syntax::Shell => &["#"][..],
        Syntax::Json | Syntax::Markdown => &[][..],
    };
    let mut result = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let rest: String = chars[index..].iter().collect();
        if comment_markers
            .iter()
            .any(|marker| rest.starts_with(marker))
        {
            result.push_str(theme.comment());
            result.extend(chars[index..].iter());
            result.push_str(theme.screen());
            break;
        }
        let ch = chars[index];
        if ch == '"' || ch == '\'' || ch == '`' {
            let quote = ch;
            let start = index;
            index += 1;
            while index < chars.len() {
                if chars[index] == quote && chars[index.saturating_sub(1)] != '\\' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            result.push_str(theme.string());
            result.extend(chars[start..index].iter());
            result.push_str(theme.screen());
        } else if ch.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
                index += 1;
            }
            result.push_str(theme.number());
            result.extend(chars[start..index].iter());
            result.push_str(theme.screen());
        } else if ch.is_alphanumeric() || ch == '_' {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            if keywords.split_whitespace().any(|keyword| keyword == word) {
                result.push_str(theme.keyword());
                result.push_str(&word);
                result.push_str(theme.screen());
            } else {
                result.push_str(&word);
            }
        } else {
            result.push(ch);
            index += 1;
        }
    }
    result
}

fn render_cursor_line(
    line: &str,
    cursor_col: usize,
    syntax: Option<Syntax>,
    theme: Theme,
) -> String {
    let cursor_col = min(cursor_col, line.len());
    let Some(ch) = line[cursor_col..].chars().next() else {
        return format!(
            "{}\x1b[7m {}\x1b[0m{}",
            highlight_syntax(line, syntax, theme),
            theme.screen(),
            theme.screen()
        );
    };
    let end = cursor_col + ch.len_utf8();
    let prefix = &line[..cursor_col];
    let suffix = &line[end..];
    format!(
        "{}\x1b[7m{}\x1b[0m{}{}",
        highlight_syntax(prefix, syntax, theme),
        ch,
        theme.screen(),
        highlight_syntax(suffix, syntax, theme)
    )
}

fn truncate_message(message: &str, max_len: usize) -> String {
    let mut result = message.chars().take(max_len).collect::<String>();
    if message.chars().count() > max_len {
        result.push('…');
    }
    result
}

fn strip_code_fence(response: &str) -> &str {
    let trimmed = response.trim();
    let Some(after_opening) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let after_language = after_opening
        .split_once('\n')
        .map(|(_, content)| content)
        .unwrap_or(after_opening);
    after_language.strip_suffix("```").unwrap_or(after_language)
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = min(index, text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    index = min(index, text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn read_key() -> io::Result<Key> {
    static IGNORE_NEXT_LF: AtomicBool = AtomicBool::new(false);

    let mut byte = [0_u8; 1];
    loop {
        if io::stdin().read(&mut byte)? == 1 {
            if byte[0] == 10 && IGNORE_NEXT_LF.swap(false, Ordering::Relaxed) {
                continue;
            }
            if byte[0] != 10 {
                IGNORE_NEXT_LF.store(false, Ordering::Relaxed);
            }
            break;
        }
    }
    match byte[0] {
        3 => Ok(Key::CtrlC),
        9 => Ok(Key::Tab),
        13 => {
            IGNORE_NEXT_LF.store(true, Ordering::Relaxed);
            Ok(Key::Enter)
        }
        10 => Ok(Key::Enter),
        27 => read_escape_sequence(),
        127 | 8 => Ok(Key::Backspace),
        b => Ok(std::str::from_utf8(&[b])
            .ok()
            .and_then(|s| s.chars().next())
            .map(Key::Char)
            .unwrap_or(Key::Unknown)),
    }
}

fn read_escape_sequence() -> io::Result<Key> {
    let mut seq = [0_u8; 2];
    if io::stdin().read(&mut seq[..1])? == 0 {
        return Ok(Key::Esc);
    }
    if seq[0] != b'[' {
        return Ok(Key::Esc);
    }
    if io::stdin().read(&mut seq[1..2])? == 0 {
        return Ok(Key::Esc);
    }
    match seq[1] {
        b'A' => Ok(Key::ArrowUp),
        b'B' => Ok(Key::ArrowDown),
        b'C' => Ok(Key::ArrowRight),
        b'D' => Ok(Key::ArrowLeft),
        _ => Ok(Key::Unknown),
    }
}

fn run_stty(args: &[&str]) -> io::Result<()> {
    let status = Command::new("stty").args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("stty failed"))
    }
}

fn terminal_size() -> (usize, usize) {
    let output = Command::new("stty").arg("size").output();
    let Some(stdout) = output.ok().filter(|output| output.status.success()) else {
        return (24, 80);
    };
    let output = String::from_utf8_lossy(&stdout.stdout);
    let mut dimensions = output.split_whitespace();
    let rows = dimensions.next().and_then(|value| value.parse().ok());
    let cols = dimensions.next().and_then(|value| value.parse().ok());
    (rows.unwrap_or(24), cols.unwrap_or(80))
}

fn effective_render_dimensions(
    pro_active: bool,
    configured_rows: Option<usize>,
    configured_cols: Option<usize>,
    terminal_rows: usize,
    terminal_cols: usize,
) -> (usize, usize) {
    if !pro_active {
        return (terminal_rows, terminal_cols);
    }
    (
        configured_rows.unwrap_or(terminal_rows).min(terminal_rows),
        configured_cols.unwrap_or(terminal_cols).min(terminal_cols),
    )
}

fn truncate_terminal_line(line: &str, max_chars: usize) -> String {
    if visible_width(line) <= max_chars {
        return line.to_owned();
    }

    let mut result = String::new();
    let mut chars = line.chars().peekable();
    let content_width = max_chars.saturating_sub(1);
    let mut visible_chars = 0;
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            result.push(ch);
            if chars.peek() == Some(&'[') {
                result.push(chars.next().expect("peeked ANSI sequence"));
                for sequence_char in chars.by_ref() {
                    result.push(sequence_char);
                    if ('@'..='~').contains(&sequence_char) {
                        break;
                    }
                }
            }
            continue;
        }
        if visible_chars >= content_width {
            break;
        }
        result.push(ch);
        visible_chars += 1;
    }
    if max_chars > 0 {
        result.push('…');
    }
    result
}

fn visible_width(line: &str) -> usize {
    let mut chars = line.chars().peekable();
    let mut width = 0;
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for sequence_char in chars.by_ref() {
                if ('@'..='~').contains(&sequence_char) {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

fn open_external(path: &Path) -> io::Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener).arg(path).spawn()?.wait()?;
    Ok(())
}

fn split_editor_lines(content: &str) -> Vec<String> {
    let mut lines: Vec<String> = content
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect();
    if content.ends_with('\n') {
        let _ = lines.pop();
    }
    lines
}

fn serialize_editor_lines(lines: &[String]) -> String {
    format!("{}\n", lines.join("\n"))
}

fn first_non_blank(line: &str) -> usize {
    line.char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn is_autocorrect_separator(ch: char) -> bool {
    matches!(ch, '.' | ',' | '!' | '?' | ':' | ';' | ')' | ']' | '}')
}

fn autocorrect_text(
    text: &str,
    replacements: &std::collections::BTreeMap<String, String>,
) -> (String, usize) {
    let mut result = String::with_capacity(text.len());
    let mut word = String::new();
    let mut changes = 0;

    let flush_word = |result: &mut String, word: &mut String, changes: &mut usize| {
        if word.is_empty() {
            return;
        }
        if let Some(replacement) = replacements.get(word) {
            result.push_str(replacement);
            *changes += 1;
        } else {
            result.push_str(word);
        }
        word.clear();
    };

    for ch in text.chars() {
        if is_word_char(ch) {
            word.push(ch);
        } else {
            flush_word(&mut result, &mut word, &mut changes);
            result.push(ch);
        }
    }
    flush_word(&mut result, &mut word, &mut changes);
    (result, changes)
}

fn parse_command_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn render_markdown(lines: &[String], width: usize) -> Vec<String> {
    let mut output = Vec::new();
    let mut in_code_block = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            output.push(String::from("────────────────────────"));
            continue;
        }
        if in_code_block {
            output.push(format!("  {line}"));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("### ") {
            output.push(format!("  {}", heading.to_ascii_uppercase()));
        } else if let Some(heading) = trimmed.strip_prefix("## ") {
            output.push(format!(" {}", heading.to_ascii_uppercase()));
            output.push(String::from(" ───────────────────────"));
        } else if let Some(heading) = trimmed.strip_prefix("# ") {
            output.push(heading.to_ascii_uppercase());
            output.push("═".repeat(min(width, heading.chars().count().max(8))));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            output.push(format!("  • {}", strip_markdown_inline(item)));
        } else if let Some(quote) = trimmed.strip_prefix("> ") {
            output.push(format!("  │ {}", strip_markdown_inline(quote)));
        } else if trimmed.is_empty() {
            output.push(String::new());
        } else {
            output.extend(wrap_text(&strip_markdown_inline(trimmed), width));
        }
    }
    output
}

fn strip_markdown_inline(input: &str) -> String {
    input
        .replace("**", "")
        .replace("__", "")
        .replace(['`', '*', '_'], "")
}

fn wrap_text(input: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in input.split_whitespace() {
        let additional = word.chars().count() + usize::from(!current.is_empty());
        if !current.is_empty() && current.chars().count() + additional > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn next_char_at(chars: &[(usize, char)], pos: usize) -> Option<&(usize, char)> {
    chars.iter().find(|(byte, _)| *byte >= pos)
}

fn line_marker(cursor: bool, selected: bool) -> &'static str {
    match (cursor, selected) {
        (true, true) => ">*",
        (true, false) => "> ",
        (false, true) => " *",
        (false, false) => "  ",
    }
}

fn main() -> io::Result<()> {
    let mut editor = match env::args_os().nth(1) {
        Some(path) => Editor::open(PathBuf::from(path))?,
        None => Editor::open_welcome()?,
    };
    let _raw = RawTerminal::enter(editor.use_alternate_buffer)?;

    loop {
        editor.render()?;
        if editor.handle_key(read_key()?)? {
            break;
        }
    }

    println!("\x1b[2J\x1b[HBye.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        autocorrect_text, ceil_char_boundary, effective_render_dimensions, env_value_is_enabled,
        floor_char_boundary, highlight_syntax, parse_command_words, render_markdown,
        serialize_editor_lines, split_editor_lines, strip_code_fence, syntax_for_path,
        truncate_terminal_line, visible_width, PathBuf, Syntax, Theme,
    };
    use std::collections::BTreeMap;

    #[test]
    fn pro_env_accepts_common_enabled_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " enabled "] {
            assert!(env_value_is_enabled(value), "{value} should enable Pro");
        }
    }

    #[test]
    fn pro_env_rejects_disabled_or_unknown_values() {
        for value in ["", "0", "false", "no", "off", "pro"] {
            assert!(
                !env_value_is_enabled(value),
                "{value} should not enable Pro"
            );
        }
    }

    #[test]
    fn pro_render_area_can_be_configured_but_free_uses_terminal() {
        assert_eq!(
            effective_render_dimensions(false, Some(10), Some(20), 40, 120),
            (40, 120)
        );
        assert_eq!(
            effective_render_dimensions(true, Some(100), Some(20), 40, 120),
            (40, 20)
        );
    }

    #[test]
    fn syntax_detection_uses_file_extension() {
        assert!(matches!(
            syntax_for_path(&PathBuf::from("main.rs")),
            Some(Syntax::Rust)
        ));
        assert!(matches!(
            syntax_for_path(&PathBuf::from("script.py")),
            Some(Syntax::Python)
        ));
        assert!(syntax_for_path(&PathBuf::from("notes.txt")).is_none());
    }

    #[test]
    fn syntax_highlighting_adds_ansi_colors() {
        let rendered = highlight_syntax("fn main() { 42 }", Some(Syntax::Rust), Theme::TokyoNight);
        assert!(rendered.contains("\x1b[38;2;187;154;247mfn"));
        assert!(rendered.contains("\x1b[38;2;255;158;100m42"));
    }

    #[test]
    fn terminal_truncation_ignores_ansi_sequences() {
        let rendered = truncate_terminal_line("\x1b[31mabcdef\x1b[0m", 4);
        assert_eq!(visible_width(&rendered), 4);
        assert!(rendered.contains("abc…"));
    }

    #[test]
    fn strip_code_fence_removes_markdown_wrapper() {
        assert_eq!(
            strip_code_fence("```rust\nfn main() {}\n```"),
            "fn main() {}\n"
        );
        assert_eq!(strip_code_fence("plain text"), "plain text");
    }

    #[test]
    fn char_boundary_helpers_handle_utf8() {
        let text = "aЖb";
        assert_eq!(floor_char_boundary(text, 2), 1);
        assert_eq!(ceil_char_boundary(text, 2), 3);
    }

    #[test]
    fn split_editor_lines_handles_lf_and_final_newline() {
        assert_eq!(
            split_editor_lines("one\n\ntwo\n"),
            vec!["one".to_string(), String::new(), "two".to_string()]
        );
    }

    #[test]
    fn split_editor_lines_normalizes_crlf_only_at_line_end() {
        assert_eq!(
            split_editor_lines("one\r\ntw\ro\r\n"),
            vec!["one".to_string(), "tw\ro".to_string()]
        );
    }

    #[test]
    fn serialize_editor_lines_preserves_blank_lines() {
        let lines = vec!["one".to_string(), String::new(), "two".to_string()];

        assert_eq!(serialize_editor_lines(&lines), "one\n\ntwo\n");
    }

    #[test]
    fn autocorrect_replaces_complete_words_only() {
        let replacements = BTreeMap::from([
            (String::from("teh"), String::from("the")),
            (String::from("превет"), String::from("привет")),
        ]);
        assert_eq!(
            autocorrect_text("teh theme, превет!", &replacements),
            (String::from("the theme, привет!"), 2)
        );
    }

    #[test]
    fn markdown_preview_formats_common_blocks() {
        let rendered = render_markdown(
            &[
                String::from("# Title"),
                String::from("- **item**"),
                String::from("> quote"),
            ],
            40,
        );
        assert_eq!(rendered[0], "TITLE");
        assert_eq!(rendered[2], "  • item");
        assert_eq!(rendered[3], "  │ quote");
    }

    #[test]
    fn mcp_command_parser_supports_quoted_arguments() {
        assert_eq!(
            parse_command_words("docs npx -y '@model/context server'"),
            vec!["docs", "npx", "-y", "@model/context server"]
        );
    }
}
