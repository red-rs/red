use std::collections::HashMap;
use crossterm::style::Color;
use serde_yaml::Value;
use std::time::Instant;
use std::collections::VecDeque;

pub fn hex_to_color(hex_color: &str) -> Color {
    let hex = hex_color.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    Color::Rgb { r, g, b }
}

pub fn hex_to_rgb(hex_color: &str) -> (u8, u8, u8) {
    let hex = hex_color.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    (r, g, b)
}

pub fn yaml_to_map(yaml: Value) -> HashMap<String, String> {
    yaml.as_mapping()
        .map(|mapping| {
            mapping.into_iter()
                .filter_map(|(key, value)| {
                    key.as_str().and_then(|k| {
                        value.as_str().map(|v| (k.to_string(), v.to_string()))
                    })
                })
                .collect()
        })
        .unwrap_or_else(HashMap::new)
}

pub fn abs_file(input: &str) -> String {
    let srcdir = std::path::PathBuf::from(input);
    let c = std::fs::canonicalize(&srcdir).unwrap();
    c.to_string_lossy().to_string()
}
pub fn get_file_name(input: &str) -> String {
    let path_buf = std::path::PathBuf::from(input);
    let file_name = path_buf.file_name().unwrap().to_string_lossy().into_owned();
    file_name
}

pub fn score_matches(src: &str, match_str: &str) -> i32 {
    let mut score = 0;

    // If the match is at the beginning, we give it a high score.
    if src.starts_with(match_str) {
        score += 1000;
    }

    // Each occurrence of match_str in src adds a smaller score.
    score += (src.matches(match_str).count() as i32) * 10;

    // If match is close to the start of the string but not at the beginning, add some score.
    if let Some(initial_index) = src.find(match_str) {
        if initial_index > 0 && initial_index < 5 {
            score += 500;
        }
    }

    score
}

const WORD_BREAK_CHARS: [char; 23] = [
    ' ', '.', ',', '=', '+', '-', '[', '(', '{', ']', ')', '}', 
    '"', ':', '&', '?', '!', ';', '\t', '/', '<', '>', '\n'
];

pub fn find_next_word(line: &str, from: usize) -> usize {
    // Find the next word index after the specified index
    let chars: Vec<char> = line.chars().collect();
    for i in from..chars.len() {
        if WORD_BREAK_CHARS.contains(&chars[i]) {
            return i;
        }
    }
    chars.len()-1
}

pub fn find_prev_word(line: &str, from: usize) -> usize {
    // Find the previous word index before the specified index
    let chars: Vec<char> = line.chars().collect();
    for i in (0..from).rev() {
        let ch = match chars.get(i) {
            Some(ch) => ch,
            None => { return 0; },
        };

        if WORD_BREAK_CHARS.contains(ch) {
            return i + 1;
        }
    }
    0
}

pub fn pad_left(str: &str, length: usize) -> String {
    format!("{:1$}", str, length)
}

pub const IGNORE_DIRS: &[&str] = &[
    ".git",
    ".idea",
    ".vscode",
    "node_modules",
    "dist",
    "target",
    "__pycache__",
    ".pytest_cache",
    "build",
    ".DS_Store",
    ".venv",
    "venv",
];

pub const IGNORE_FILES: &[&str] = &[
    ".DS_Store",
];

pub const IGNORE_EXTS: &[&str] = &[
    "doc", "docx", "pdf", "rtf", "odt", "xlsx", "pptx", "jpg", "png", "gif", "bmp", "svg",
    "tiff", "mp3", "wav", "aac", "flac", "ogg", "mp4", "avi", "mov", "wmv", "mkv", "zip",
    "rar", "tar.gz", "7z", "exe", "msi", "bat", "sh", "so", "ttf", "otf",
];

pub fn current_dir() -> String {
    std::env::current_dir().unwrap()
        .to_string_lossy().into_owned()
}

pub fn current_directory_name() -> Option<String> {
    if let Ok(current_dir) = std::env::current_dir() {
        if let Some(dir_name) = current_dir.file_name() {
            return dir_name.to_str().map(String::from);
        }
    }
    None
}


#[derive(Clone)]
pub struct CursorPosition {
    pub filename: String,
    pub row: usize,
    pub col: usize,
    pub y: usize,
    pub x: usize,
}

pub struct CursorHistory {
    index: usize,
    max_items: usize,
    positions: VecDeque<CursorPosition>,
}

impl CursorHistory {
    pub fn new() -> Self {
        Self {
            index: 0,
            max_items: 10000,
            positions: VecDeque::new(),
        }
    }

    pub fn push(&mut self, position: CursorPosition) {
        // Do not store if prev position the sam
        if self.index > 0 {
            if let Some(last) = self.positions.get(self.index - 1) {
                if last.row == position.row
                    && last.col == position.col
                    && last.filename == position.filename
                {
                    return; 
                }
            }
        }
        
        // Remove all redo history
        while self.positions.len() > self.index {
            self.positions.pop_back();
        }

        // Limit max history size
        if self.positions.len() == self.max_items {
            self.positions.pop_front();
            self.index -= 1;
        }

        self.positions.push_back(position);
        self.index += 1;
    }

    pub fn undo(&mut self) -> Option<CursorPosition> {
        if self.index == 0 {
            None
        } else {
            self.index -= 1;
            self.positions.get(self.index).cloned()
        }
    }

    pub fn redo(&mut self) -> Option<CursorPosition> {
        if self.index >= self.positions.len() {
            None
        } else {
            let pos = self.positions.get(self.index).cloned();
            self.index += 1;
            pos
        }
    }

    pub fn current(&self) -> Option<&CursorPosition> {
        if self.index == 0 {
            None
        } else {
            self.positions.get(self.index - 1)
        }
    }
    
    pub fn peek_redo(&self) -> Option<&CursorPosition> {
        self.positions.get(self.index)
    }
}


struct Cell {
    character: char,
    fg_color: crossterm::style::Color,
    bg_color: crossterm::style::Color,
}

struct ScreenBuffer {
    width: usize,
    height: usize,
    cells: Vec<Vec<Cell>>,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickType {
    Single,
    Double,
    Triple,
}

impl ClickType {
    pub fn from_click_history(
        now: Instant,
        cursor: usize,
        last_click: Option<(Instant, usize)>,
        last_last_click: Option<(Instant, usize)>,
    ) -> Self {
        let max_dt = std::time::Duration::from_millis(700);

        let is_double = match last_click {
            Some((t1, c1)) => c1 == cursor && now.duration_since(t1) < max_dt,
            None => false,
        };

        let is_triple = match (last_click, last_last_click) {
            (Some((t1, c1)), Some((t0, c0))) => {
                c0 == cursor && c1 == cursor &&
                now.duration_since(t0) < max_dt &&
                t1.duration_since(t0) < max_dt
            }
            _ => false,
        };

        match (is_triple, is_double) {
            (true, _) => ClickType::Triple,
            (false, true) => ClickType::Double,
            _ => ClickType::Single,
        }
    }
}

