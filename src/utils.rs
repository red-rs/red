// utils.rs
use std::collections::HashMap;
use crossterm::style::Color;
use serde_yaml::Value;

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


pub struct CursorPosition {
    pub filename: String,
    pub row: usize,
    pub col: usize,
    pub y: usize,
    pub x: usize,
}
pub struct CursorHistory {
    positions: Vec<CursorPosition>
}

impl CursorHistory {
    pub fn new() -> Self { Self { positions: Vec::new()} }

    pub fn push(&mut self, cp: CursorPosition) {
        self.positions.push(cp);
    }

    pub fn pop(&mut self) -> Option<CursorPosition> {
        self.positions.pop()
    }
    pub fn clear(&mut self) {
        self.positions.clear();
    }
}