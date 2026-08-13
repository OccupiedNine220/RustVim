use std::{
    cmp::{max, min},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

const PRO_ENV_VAR: &str = "RUSTVIM_PRO";
const AI_COMMAND_ENV_VAR: &str = "RUSTVIM_AI_COMMAND";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Insert,
    VisualLine,
    Command,
}

enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    CtrlC,
    Unknown,
}

struct RawTerminal;

impl RawTerminal {
    fn enter() -> io::Result<Self> {
        run_stty(&["raw", "-echo", "min", "0", "time", "1"])?;
        print!("\x1b[?1049h\x1b[?25l");
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
}

impl Editor {
    fn open(path: PathBuf) -> io::Result<Self> {
        Self::open_with_pro(path, pro_active_from_env())
    }

    fn open_with_pro(path: PathBuf, pro_active: bool) -> io::Result<Self> {
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

        Ok(Self {
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
            show_numbers: true,
            use_alternate_buffer: true,
            pro_active,
            syntax_enabled: pro_active,
        })
    }

    fn render(&self) -> io::Result<()> {
        print!("\x1b[2J\x1b[H");
        let (sel_start, sel_end) = self.selection_bounds();
        let number_width = max(4, self.lines.len().to_string().len());
        for (index, line) in self.lines.iter().enumerate() {
            let selected = self.mode == Mode::VisualLine && (sel_start..=sel_end).contains(&index);
            let cursor = index == self.cursor_line;
            let marker = line_marker(cursor, selected);
            let rendered_line = if cursor {
                render_cursor_line(
                    line,
                    self.cursor_col,
                    syntax_for_path(&self.path).filter(|_| self.syntax_active()),
                )
            } else if self.syntax_active() {
                highlight_syntax(line, syntax_for_path(&self.path))
            } else {
                line.to_owned()
            };
            if self.show_numbers {
                print!(
                    "{marker} {:>width$} | {rendered_line}\r\n",
                    index + 1,
                    width = number_width
                );
            } else {
                print!("{marker} {rendered_line}\r\n");
            }
        }
        print!("~\r\n");
        print!("~\r\n");

        let mode = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::VisualLine => "VISUAL LINE",
            Mode::Command => "COMMAND",
        };
        print!(
            "\x1b[7m {}{}  {}  line {}, col {} \x1b[0m\r\n",
            self.path.display(),
            if self.dirty { " [+]" } else { "" },
            mode,
            self.cursor_line + 1,
            self.cursor_col + 1
        );
        if self.mode == Mode::Command {
            print!("{}{}", self.command_prompt, self.command);
        } else {
            print!("{}", self.message);
        }
        io::stdout().flush()
    }

    fn handle_key(&mut self, key: Key) -> io::Result<bool> {
        match self.mode {
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
        }
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
            "help" => {
                self.message = String::from(
                    "i/a/I/A/o/O | arrows | hjkl Pro | V y d | dd yy cc p x r D C J u w b e n N >> << | / :s :set :e :w :q :ai",
                );
            }
            "set syntax" if self.pro_active => {
                self.syntax_enabled = true;
                self.message = String::from("Syntax highlighting enabled.");
            }
            "set nosyntax" => {
                self.syntax_enabled = false;
                self.message = String::from("Syntax highlighting disabled.");
            }
            other if other == "ai" || other.starts_with("ai ") => {
                self.run_ai(other.strip_prefix("ai ").unwrap_or("Summarize this file"))?;
            }
            "ai-summary" => self.run_ai("Summarize this file")?,
            "set number" | "set nu" => {
                self.show_numbers = true;
                self.message = String::from("Line numbers enabled.");
            }
            "set nonumber" | "set nonu" => {
                self.show_numbers = false;
                self.message = String::from("Line numbers disabled.");
            }
            "set altbuffer" | "set alternative-buffer" => {
                self.set_alternate_buffer(true)?;
            }
            "set noaltbuffer" | "set noalternative-buffer" => {
                self.set_alternate_buffer(false)?;
            }
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
            other if other.starts_with("term ") => {
                self.open_terminal(Some(other[5..].trim()))?;
            }
            other if other.starts_with("%s/") || other.starts_with("s/") => self.substitute(other),
            _ => self.message = format!("Unknown command: :{command}"),
        }
        Ok(false)
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
        self.path = path;
        self.dirty = false;
        self.message = format!("Saved as: {}", self.path.display());
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
            self.lines = lines;
            self.cursor_line = min(self.cursor_line, self.lines.len() - 1);
            self.clamp_cursor();
            self.dirty = true;
            self.message = String::from("Undo.");
        } else {
            self.message = String::from("Nothing to undo.");
        }
    }

    fn snapshot(&mut self) {
        self.undo.push(self.lines.clone());
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

    fn clamp_cursor(&mut self) {
        self.cursor_col = min(self.cursor_col, self.current_line_len());
    }

    fn current_line_len(&self) -> usize {
        self.lines[self.cursor_line].len()
    }

    fn syntax_active(&self) -> bool {
        self.pro_active && self.syntax_enabled && syntax_for_path(&self.path).is_some()
    }

    fn run_ai(&mut self, prompt: &str) -> io::Result<()> {
        if !self.pro_active {
            self.message = String::from("AI features require an active RustVim Pro subscription.");
            return Ok(());
        }
        let Ok(command) = env::var(AI_COMMAND_ENV_VAR) else {
            self.message = format!("Set {AI_COMMAND_ENV_VAR} to use :ai.");
            return Ok(());
        };
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("RUSTVIM_AI_PROMPT", prompt)
            .env("RUSTVIM_AI_FILE", self.path.to_string_lossy().as_ref())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(self.full_text().as_bytes())?;
                }
                child.wait_with_output()
            });
        match output {
            Ok(output) if output.status.success() => {
                let response = String::from_utf8_lossy(&output.stdout);
                let response = response
                    .lines()
                    .next()
                    .unwrap_or("AI returned no output")
                    .trim();
                self.message = format!("AI: {}", truncate_message(response, 180));
            }
            Ok(output) => {
                let error = String::from_utf8_lossy(&output.stderr);
                self.message = format!("AI failed: {}", truncate_message(error.trim(), 160));
            }
            Err(error) => self.message = format!("AI failed to start: {error}"),
        }
        Ok(())
    }

    fn handle_hjkl(&mut self, key: char) {
        if !self.pro_active {
            self.message = String::from(
                "h/j/k/l navigation is included in RustVim Pro. Use arrow keys for the free tier.",
            );
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
        self.message = String::from("Exit requires an active RustVim Pro subscription.");
        false
    }
}

fn pro_active_from_env() -> bool {
    env::var(PRO_ENV_VAR)
        .map(|value| env_value_is_enabled(&value))
        .unwrap_or(false)
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

fn highlight_syntax(line: &str, syntax: Option<Syntax>) -> String {
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
            result.push_str("\x1b[90m");
            result.extend(chars[index..].iter());
            result.push_str("\x1b[0m");
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
            result.push_str("\x1b[32m");
            result.extend(chars[start..index].iter());
            result.push_str("\x1b[0m");
        } else if ch.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
                index += 1;
            }
            result.push_str("\x1b[33m");
            result.extend(chars[start..index].iter());
            result.push_str("\x1b[0m");
        } else if ch.is_alphanumeric() || ch == '_' {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            if keywords.split_whitespace().any(|keyword| keyword == word) {
                result.push_str("\x1b[35m");
                result.push_str(&word);
                result.push_str("\x1b[0m");
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

fn render_cursor_line(line: &str, cursor_col: usize, syntax: Option<Syntax>) -> String {
    let cursor_col = min(cursor_col, line.len());
    let Some(ch) = line[cursor_col..].chars().next() else {
        return format!("{}\x1b[7m \x1b[0m", highlight_syntax(line, syntax));
    };
    let end = cursor_col + ch.len_utf8();
    let prefix = &line[..cursor_col];
    let suffix = &line[end..];
    format!(
        "{}\x1b[7m{}\x1b[0m{}",
        highlight_syntax(prefix, syntax),
        ch,
        highlight_syntax(suffix, syntax)
    )
}

fn truncate_message(message: &str, max_len: usize) -> String {
    let mut result = message.chars().take(max_len).collect::<String>();
    if message.chars().count() > max_len {
        result.push('…');
    }
    result
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
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("untitled.txt"));
    let _raw = RawTerminal::enter()?;
    let mut editor = Editor::open(path)?;

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
        env_value_is_enabled, highlight_syntax, serialize_editor_lines, split_editor_lines,
        syntax_for_path, PathBuf, Syntax,
    };

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
        let rendered = highlight_syntax("fn main() { 42 }", Some(Syntax::Rust));
        assert!(rendered.contains("\x1b[35mfn\x1b[0m"));
        assert!(rendered.contains("\x1b[33m42\x1b[0m"));
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
}
