use crossterm::style::Color;
use rayon::vec;
use ropey::iter::Lines;
use ropey::Rope;
use ropey::RopeSlice;
use tree_sitter::InputEdit;
use std::cmp::min;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, TextProvider};

use crate::config::{Config, Language};
use crate::utils::{self, hex_to_color};
use strfmt::strfmt;
use log2::*;
use streaming_iterator::StreamingIterator;

pub struct Code {
    pub file_name: String,
    pub abs_path: String,
    pub lang: String,
    pub text: ropey::Rope,
    pub changed: bool,
    pub undo_history: Vec<Change>,
    pub redo_history: Vec<Change>,
    tree: Option<tree_sitter::Tree>,
    parser: Option<tree_sitter::Parser>,
    query: Option<tree_sitter::Query>,
    r: usize, c: usize, x: usize, y: usize,
    lang_conf: Option<Language>,
    line2runneble: HashMap<usize, Runnable>,
    query_test: Option<tree_sitter::Query>,
}

impl Code {
    pub fn new() -> Self {
        Self {
            text: Rope::new(),
            file_name: String::new(),
            abs_path: String::new(),
            changed: false,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            tree: None,
            lang: String::new(),
            parser: None,
            query: None,
            r: 0, c: 0, x: 0, y: 0,
            lang_conf: None,
            line2runneble: HashMap::new(),
            query_test: None,
        }
    }

    pub fn from_str(text: &str) -> Self {
        let mut code = Self::new();
        code.insert_text(text, 0, 0);
        code
    }

    pub fn from_file(path: &str, conf: &Config) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let text = Rope::from_reader(BufReader::new(file))?;
        let abs_path = utils::abs_file(path);
        let file_name = utils::get_file_name(path);

        let mut lang = match detect_lang::from_path(path) {
            Some(lang) => lang.id().to_lowercase(),
            None => {
                // find lang by ext from config
                match conf.language.iter().find(|l| l.types.iter().any(|t| path.ends_with(t))) {
                    Some(lang) => lang.name.to_string(),
                    None => "text".to_string(),
                }
            }
        };

        let lang_conf = conf.language.iter().find(|l| l.name == lang);
        let lang_conf = lang_conf.map(|lc| (*lc).clone());

        let language = match lang.as_str() {
            "rust" => tree_sitter_rust::LANGUAGE.into(),
            "javascript" => tree_sitter_javascript::LANGUAGE.into(),
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "python" => tree_sitter_python::LANGUAGE.into(),
            "go" => tree_sitter_go::LANGUAGE.into(),
            "html" => tree_sitter_html::LANGUAGE.into(),
            "css" => tree_sitter_css::LANGUAGE.into(),
            "shell" => tree_sitter_bash::LANGUAGE.into(),
            "toml" => tree_sitter_toml_ng::LANGUAGE.into(),
            "java" => tree_sitter_java::LANGUAGE.into(),
            "cpp" => tree_sitter_cpp::LANGUAGE.into(),
            "c" => tree_sitter_c::LANGUAGE.into(),
            "zig" => tree_sitter_zig::LANGUAGE.into(),
            "lua" => tree_sitter_lua::LANGUAGE.into(),
            "json" => tree_sitter_json::LANGUAGE.into(),
            "yaml" => tree_sitter_yaml::LANGUAGE.into(),
            "toml" => tree_sitter_toml_ng::LANGUAGE.into(),

            _ => {
                lang = "text".to_string();
                tree_sitter_rust::LANGUAGE.into()
            }
        };

        if lang == "text".to_string() {
            return Ok(Self {
                text,
                file_name,
                abs_path,
                changed: false,
                undo_history: Vec::new(),
                redo_history: Vec::new(),
                tree:None,
                lang,
                parser: None,
                query: None,
                r: 0, c: 0, x: 0, y: 0,
                lang_conf,
                line2runneble: HashMap::new(),
                query_test: None,
            })
        }

        let mut parser = Parser::new();
        parser.set_language(&language).expect("Can not set_language");

        let tree = parser.parse(text.to_string(), None);

        let query_highlight_content = crate::config::get_file_content(
            format!("langs/{}/highlights.scm", lang).as_str()
        ).unwrap_or("".to_string());

        let query_highlight = match Query::new(&language, &query_highlight_content) {
            Ok(q) => Some(q),
            Err(e) => { 
                debug!("err {}", e); 
                None 
            },
        };
        
        let query_test_path = Path::new(".").join("langs")
            .join(lang.clone()).join("tests.scm");

        let query_test_content = crate::config::get_file_content(
            query_test_path.to_str().unwrap_or("")
        );

        let query_test = match query_test_content {
            Ok(query_test_content) => {
                match Query::new(&language, &query_test_content) {
                    Ok(q) => Some(q),
                    Err(e) => { debug!("err {}", e); None },
                }
            }
            Err(e) => { debug!("err {}", e); None },
        };

        let mut this = Self {
            text,
            file_name,
            abs_path,
            changed: false,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            tree,
            lang,
            parser: Some(parser),
            query: query_highlight,
            r: 0, c: 0, x: 0, y: 0,
            lang_conf,
            line2runneble: HashMap::new(),
            query_test
        };

        this.update_runnables();
        Ok(this)
    }
    
    pub fn reload(&mut self) -> std::io::Result<()>{
        let file = File::open(&self.abs_path)?;
        let text = Rope::from_reader(BufReader::new(file))?;
        
        let last_row =  self.text.len_lines() - 1;
        let last_col = self.line_len(last_row);
        
        self.replace_text(0, 0, last_row, last_col, &text.to_string());
        
        Ok(())
    }
    
    pub fn set_lang(&mut self, lang:String, conf: &Config) {
        self.lang = lang;
        let lang_conf = conf.language.iter().find(|l| l.name == self.lang);
        self.lang_conf = lang_conf.map(|lc| (*lc).clone());
    }

    pub fn set_cursor_position(&mut self, r: usize, c: usize, y: usize, x: usize) {
        self.r = r;
        self.c = c;
        self.y = y;
        self.x = x;
    }
    pub fn get_cursor_position(&mut self) -> (usize,usize,usize,usize) {
        (self.r.clone(), self.c.clone(), self.y.clone(), self.x.clone())
    }

    pub fn save_file(&mut self) -> std::io::Result<()> {
        if !self.changed { return Ok(()); }

        let file = File::create(&self.abs_path)?;
        let saved = self.text.write_to(BufWriter::new(file));
        self.changed = false;
        saved
    }

    fn insert(&mut self, text: &str, from: usize) {
        let offset_byte = self.text.char_to_byte(from);
        self.text.insert(from, text);
        self.changed = true;

        let total_bytes: usize = text.chars().map(|ch| ch.len_utf8()).sum();
        let edit = tree_sitter::InputEdit {
            start_byte: offset_byte,
            old_end_byte: offset_byte,
            new_end_byte: offset_byte + total_bytes,
            start_position: Point { row: 0, column: 0 },
            old_end_position: Point { row: 0, column: 0 },
            new_end_position: Point { row: 0, column: 0 },
        };
        debug!("ts insert {:?}", edit);
        self.apply_edit(edit);
    }

    pub fn insert_text(&mut self, text: &str, row: usize, column: usize) {
        let from = self.text.line_to_char(row) + column;
        self.insert(text, from);

        self.undo_history.push(Change {
            start: from,
            operation: Operation::Insert,
            text: text.to_string(),
            row, column
        });
        
        self.redo_history.clear();
    }

    pub fn insert_char(&mut self, c: char, row: usize, column: usize) {
        self.insert_text(&c.to_string(), row, column);
    }

    pub fn insert_tab(&mut self, row: usize, column: usize) -> String {
        let text = match self.lang_conf.as_ref() {
            Some(conf) if conf.indent.unit == " " =>
                " ".repeat(conf.indent.width as usize).to_string(),
            _ =>  "\t".to_string(),
        };

        self.insert_text(&text, row, column);
        return text;
    }

    /// removes from text and edit tree
    fn remove(&mut self, from: usize, to: usize) {
        let from_byte = self.text.char_to_byte(from);
        let to_byte = self.text.char_to_byte(to);

        self.text.remove(from..to);
        self.changed = true;

        let edit = tree_sitter::InputEdit {
            start_byte: from_byte,
            old_end_byte: to_byte,
            new_end_byte: from_byte,
            start_position: Point { row: 0, column: 0 },
            old_end_position: Point { row: 0, column: 0 },
            new_end_position: Point { row: 0, column: 0 },
        };
        debug!("ts remove {:?}", edit);
        self.apply_edit(edit);
    }

    pub fn remove_text(&mut self, row: usize, col: usize, row1: usize, col1: usize) {
        let from = self.text.line_to_char(row) + col;
        let to = self.text.line_to_char(row1) + col1;
        let text = self.text.slice(from..to).to_string();
        
        self.remove(from, to);
        
        self.undo_history.push(Change {
            start: from,
            operation: Operation::Remove,
            text: text.to_string(),
            row:row1, column:col1
        });
        
        self.redo_history.clear();
    }
    
    pub fn remove_char(&mut self, row: usize, column: usize) {
        self.remove_text(row, column-1, row, column);
    }
    
    pub fn replace_text(&mut self, row: usize, col: usize, row1: usize, col1: usize, text: &str) {
        let from = self.text.line_to_char(row) + col;
        let to = self.text.line_to_char(row1) + col1;
        let removed_text = self.text.slice(from..to).to_string();
        
        self.undo_history.push(Change {
            start: from,
            operation: Operation::Start,
            text: "".to_string(),
            row: row1, column: col1
        });
        
        self.remove_text(row, col, row1, col1);
        self.insert_text(text, row, col);
        
        self.undo_history.push(Change {
            start: from,
            operation: Operation::End,
            text: "".to_string(),
            row: row1, column: col1
        });
        
        self.redo_history.clear();
    }
    
    fn apply_edit(&mut self, edit: InputEdit) {
        match self.tree.as_mut() {
            Some(tree) => {
                tree.edit(&edit);
                self.tree_parse();
                self.update_runnables();
            },
            None => return,
        }
    }
    fn tree_parse(&mut self) {
        if let Some(parser) = &mut self.parser {
            // let text = self.text.to_string();
            let rope = &self.text;
            self.tree = parser.parse_with(&mut |byte, _| {
                // debug!("parse_with {}", byte);
                let sl = if byte <= rope.len_bytes() {
                    let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                    chunk[byte - start..].as_bytes()
                } else {
                    &[]
                };
                // debug!("sl {:?}", String::from_utf8_lossy(sl));
                sl
            }, self.tree.as_ref());

            // self.tree = parser.parse(text, self.tree.as_ref());
        }
    }

    fn set_text(&mut self, text: &str) {
        self.text = Rope::from(text);
        
        if let Some(parser) = &mut self.parser {
            let tree = parser.parse(self.text.to_string(), None);
            self.tree = tree;
            self.update_runnables();
        }
    }
    
    pub fn get_text(&mut self, row: usize, col: usize, row1: usize, col1: usize) -> String {
        let from = self.text.line_to_char(row) + col;
        let to = self.text.line_to_char(row1) + col1;
        let string = self.text.slice(from..to).to_string();
        return string;
    }

    pub fn line_len(&self, idx: usize) -> usize {
        let line = self.text.line(idx);
        let chars = line.chars();
        let len = chars.len();

        // '/n' at the end if len not 0, do not count it
        if chars.last() == Some('\n') { len - 1 } else { len }
    }

    pub fn get_line_at(&self, idx: usize) -> Option<RopeSlice> {
        self.text.get_line(idx)
    }

    pub fn line_at(&self, idx: usize) -> Option<&str> {
        let line = self.text.line(idx);
        line.as_str()
    }

    pub fn lines_from(&self, from: usize) -> Option<Lines> {
        self.text.get_lines_at(from)
    }

    pub fn get_from(&self, from: usize) -> Lines {
        self.text.lines_at(from)
    }

    pub fn from_to(&self, row: usize, col: usize, row1: usize, col1: usize) -> RopeSlice {
        let from = self.text.line_to_char(row) + col;
        let to = self.text.line_to_char(row1) + col1;
        self.text.slice(from..to)
    }

    pub fn slice(&self, from: usize, to: usize) -> RopeSlice {
        let max_index = self.text.len_lines();
        let end_index = min(to, max_index);
        let start_index = self.text.line_to_char(from);
        let end_index = self.text.line_to_char(end_index);
        self.text.slice(start_index..end_index)
    }
    pub fn char_slice(&self, start: usize, end: usize) -> RopeSlice {
        self.text.slice(start..end)
    }

    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }
    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }
    pub fn line_to_byte(&self, line: usize) -> usize {
        self.text.line_to_byte(line)
    }
    pub fn line_to_char(&self, line_idx: usize) -> usize {
        self.text.line_to_char(line_idx)
    }
    pub fn point(&self, offset: usize) -> (usize, usize) {
        let row = self.text.char_to_line(offset);
        let line_start = self.text.line_to_char(row);
        let col = offset - line_start;
        (row, col)
    }

    pub fn offset(&self, row: usize, col: usize) -> usize {
        let line_start = self.text.line_to_char(row);
        line_start + col
    }

    pub fn is_empty(&self) -> bool {
        self.text.len_chars() == 0
    }

    pub fn indentation_level(&self, line: usize) -> usize {
        match self.lang_conf.as_ref() {
            Some(conf) if conf.indent.unit == " " => {  // spaces case
                let spaces_count = self.text.line(line).chars().take_while(|&ch| ch == ' ').count();
                // println!("spaces_count={spaces_count}");
                let width = if conf.indent.width > 0 { conf.indent.width as usize} else { 2 };
                return (spaces_count as f64 / width as f64).ceil() as usize;
            }
            _ =>  {
                let tabs_count = self.text.line(line).chars().take_while(|&ch| ch == '\t').count();
                // println!("tabs_count={tabs_count}");
                return tabs_count;
            },
        };
    }

    fn position_to_point(text: &RopeSlice, pos: usize) -> (usize, usize) {
        let byte = text.char_to_byte(pos);
        let line = text.char_to_line(pos);
        let line_start_byte = text.line_to_byte(line);
        let col = byte - line_start_byte;

        (line, col)
    }

    pub fn search(&self, substring: &str) -> Vec<(usize, usize)> {
        let rope_slice = &self.text.slice(0..);
        let result: Vec<(usize, usize)> = SearchIter::from_rope_slice(rope_slice, substring)
            .map(|(pos, _)| Self::position_to_point(rope_slice, pos))
            .collect();

        result
    }

    pub fn find_substring(&self, line:usize, substring: &str) -> Option<usize> {
        match self.text.get_line(line) {
            Some(line) => {
                let search_iter = EarlyTerminationSearch::from_rope_slice(&line, substring);
                let result: Vec<(usize, usize)> = search_iter.collect();
                result.first().map(|r|r.0)
            },
            None => None,
        }
    }

    pub fn find_comment(&self, line:usize) -> Option<usize> {
        match self.lang_conf.as_ref() {
            Some(conf) => self.find_substring(line, &conf.comment),
            None => None,
        }
    }

    pub fn get_lang_comment(&self) -> Option<String> {
        match self.lang_conf.as_ref() {
            Some(conf) => Some(conf.comment.clone()),
            None => None,
        }
    }
    pub fn get_lang_conf(&self) -> Option<&Language> {
        self.lang_conf.as_ref()
    }

    pub fn find_first_non_whitespace(&self, line_index:usize, stop_index: usize) -> Option<usize> {
        match self.text.get_line(line_index) {
            Some(line) => {
                line.chars()
                    .enumerate()
                    .take(stop_index)
                    .find(|(_, ch)| !ch.is_whitespace())
                    .map(|(index, _)| index)
            },
            None => None,
        }
    }

    pub fn indent_string(&self) -> Option<String> {
        match self.lang_conf.as_ref() {
            Some(conf) => Some(conf.indent.unit.repeat(conf.indent.width as usize)),
            None => None,
        }
    }
    pub fn indent_width(&self) -> Option<usize> {
        match self.lang_conf.as_ref() {
            Some(conf) => Some(conf.indent.width as usize),
            None => None,
        }
    }
    pub fn indent_unit(&self) -> Option<&String> {
        match self.lang_conf.as_ref() {
            Some(conf) => Some(&conf.indent.unit),
            None => None,
        }
    }
    pub fn is_only_indentation_before(&self, r: usize, c: usize) -> bool {
        if r >= self.text.len_lines() || c == 0 { return false; }

        let line = self.text.line(r);

        let mut col = 0;
        for ch in line.chars() {
            if col >= c { break; } // Reached the specified column
            // Found a non-whitespace character before the specified position
            if !ch.is_whitespace() { return false; }
            col += 1;
        }
        true
    }

    pub fn count_tabs(&self, line_index:usize, stop_index: usize) -> Option<usize> {
        match self.text.get_line(line_index) {
            Some(line) => {
                let mut count = 0;
                for ch in line.chars().take(stop_index) {
                    if ch == '\t' {
                        count += 1;
                    }
                }
                Some(count)
            },
            None => None,
        }
    }

    /// calculates color ranges from line number `from` to `to`
    /// returns colors vectors of (start_byte, end_byte, color): (usize, usize, Color)
    pub fn colors(
        &self,
        from: usize,
        to: usize,
        theme: &HashMap<String, String>,
    ) -> Vec<(usize, usize, usize, usize, Color)> {

        let query = match self.query.as_ref() {
            Some(q) => q, None => return Vec::new(),
        };

        let mut query_cursor = QueryCursor::new();
        let start_index = self.text.line_to_byte(from);
        let max_index = self.text.len_lines();
        let end_index = self.text.line_to_byte(min(to, max_index));

        query_cursor.set_byte_range(start_index..end_index); //superfast

        let root = self.tree.as_ref().unwrap().root_node();

        let mut query_matches = query_cursor.matches(&query, root, RopeProvider(self.text.slice(..)));
        
        let mut result = Vec::new();

        while let Some(m) = query_matches.next() {
            for capture in m.captures {
                let capture_index = capture.index as usize;
                let capture_name = &query.capture_names()[capture_index];
                let name = capture_name.split('.').next().unwrap_or(capture_name);
                let theme_colors = theme.get(name);
                let color = theme_colors
                    .map(|s| hex_to_color(s))
                    .unwrap_or(Color::Reset);

                // let node_text = self.text.byte_slice(
                //     capture.node.start_byte()..capture.node.end_byte()
                // ).as_str().unwrap_or_default(); // debug

                if color == Color::Reset { continue }

                result.push((
                    capture.node.start_byte(), 
                    capture.node.end_byte(), 
                    capture.node.start_position().row, 
                    capture.node.end_position().row, 
                    color
                ));
            }
        }
        
        result

    }

    fn update_runnables(&mut self) {
        if self.lang_conf.is_none() {return; }

        let lang = self.lang_conf.as_ref().unwrap();

        self.line2runneble.clear();

        match (lang.executable.as_ref(), lang.exec.as_ref()) {
            (Some(true), Some(template)) => {

                let mut vars = std::collections::HashMap::new();
                vars.insert("file".to_string(), self.abs_path.clone());

                let res = strfmt(&template, &vars);
                if res.is_ok() {
                    let cmd = res.unwrap();
                    let runnable = Runnable { cmd, row: 0 };
                    self.line2runneble.insert(0, runnable);
                }
            }
            _ => {},
        }

        match (lang.executable.as_ref(), lang.exectest.as_ref()) {
            (Some(true), Some(template)) => {
                if self.query_test.is_none() { return; }

                let query = self.query_test.as_ref().expect("cant get query");
                let mut query_cursor = QueryCursor::new();

                let root = self.tree.as_ref().unwrap().root_node();
                let mut matches = query_cursor.matches(&query, root, RopeProvider(self.text.slice(..)));

                while let Some(m) = matches.next() {
                    for capture in m.captures {
                        let capture_index = capture.index as usize;
                        let capture_name = &query.capture_names()[capture_index];
                        let name = capture_name.split('.').next().unwrap_or(capture_name);
                        let text = self.text.byte_slice(capture.node.start_byte()..capture.node.end_byte()).to_string();
                        let row = capture.node.start_position().row;
                        let mut vars = std::collections::HashMap::new();
                        vars.insert("test".to_string(), text);
                        vars.insert("file".to_string(), self.abs_path.clone());

                        let res = strfmt(&template, &vars);
                        if res.is_ok() {
                            let cmd = res.unwrap();
                            let runnable = Runnable { cmd, row };
                            self.line2runneble.insert(row, runnable);
                        }
                    }
                }

            }
            _ => {},
        }
    }

    pub fn is_runnable(&self, line: usize) -> bool {
        self.line2runneble.contains_key(&line)
    }
    pub fn get_runnable(&self, line: usize) -> Option<&Runnable> {
        self.line2runneble.get(&line)
    }

    
    pub fn get_node_path(
        &self, row: usize, column: usize
    ) -> Option<NodePath> {

        // return node path at row column position
        let root = self.tree.as_ref()?.root_node();
        let mut node = root.named_descendant_for_point_range(
            Point { row, column }, Point { row, column }
        );
        
        let mut path = NodePath { row, column, nodes: vec![], current:0 };

        // traverse tree to up
        while node.is_some() {
            match node {
                Some(n) => {
                    path.nodes.push((n.start_position(), n.end_position()));
                    node = n.parent();
                },
                None => { break },
            }
        }
        Some(path)
    }

}

pub struct NodePath {
    pub row: usize,
    pub column: usize,
    pub nodes: Vec<(Point,Point)>,
    current: usize
}

impl NodePath {
    pub fn current_node(&self) -> Option<&(Point, Point)>{
        self.nodes.get(self.current)
    }
    pub fn next_node(&mut self) -> Option<&(Point, Point)>{
        self.current += 1;
        if self.current >= self.nodes.len() { self.current = self.nodes.len() -1 } 
        self.nodes.get(self.current)
    }
    pub fn prev_node(&mut self) -> Option<&(Point, Point)>{
        if self.current == 0 { return None } 
        if self.current > 0 { self.current -= 1 } 
        self.nodes.get(self.current)
    }
}

pub struct Runnable {
    pub cmd: String,
    pub row: usize,
}

pub struct ChunksBytes<'a> {
    chunks: ropey::iter::Chunks<'a>,
}

impl<'a> Iterator for ChunksBytes<'a> {
    type Item = &'a [u8];
    
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(str::as_bytes)
    }
}

pub struct RopeProvider<'a>(pub RopeSlice<'a>);
impl<'a> TextProvider<&'a [u8]> for RopeProvider<'a> {
    type I = ChunksBytes<'a>;

    #[inline]
    fn text(&mut self, node: Node) -> Self::I {
        let fragment = self.0.byte_slice(node.start_byte()..node.end_byte());
        ChunksBytes {
            chunks: fragment.chunks(),
        }
    }
}

struct SearchIter<'a> {
    char_iter: ropey::iter::Chars<'a>,
    search_pattern: &'a str,
    search_pattern_char_len: usize,

    // The current char index of the search head.
    cur_index: usize,

    // Tracks where we are in the search pattern for the current possible matches.
    possible_matches: Vec<std::str::Chars<'a>>,
}

impl<'a> SearchIter<'a> {
    fn from_rope_slice<'b>(slice: &'b RopeSlice, search_pattern: &'b str) -> SearchIter<'b> {
        assert!(
            !search_pattern.is_empty(),
            "Can't search using an empty search pattern."
        );
        SearchIter {
            char_iter: slice.chars(),
            search_pattern,
            search_pattern_char_len: search_pattern.chars().count(),
            cur_index: 0,
            possible_matches: Vec::new(),
        }
    }
}

impl<'a> Iterator for SearchIter<'a> {
    type Item = (usize, usize);

    // Return the start/end char indices of the next match.
    fn next(&mut self) -> Option<(usize, usize)> {
        #[allow(clippy::while_let_on_iterator)]
        while let Some(next_char) = self.char_iter.next() {
            self.cur_index += 1;

            // Push new potential match, for a possible match starting at the
            // current char.
            self.possible_matches.push(self.search_pattern.chars());

            // Check the rope's char against the next character in each of
            // the potential matches, removing the potential matches that
            // don't match.  We're using indexing instead of iteration here
            // so that we can remove the possible matches as we go.
            let mut i = 0;
            while i < self.possible_matches.len() {
                let pattern_char = self.possible_matches[i].next().unwrap();
                if next_char == pattern_char {
                    if self.possible_matches[i].clone().next() == None {
                        // We have a match!  Reset possible matches and
                        // return the successful match's char indices.
                        let char_match_range = (
                            self.cur_index - self.search_pattern_char_len,
                            self.cur_index,
                        );
                        self.possible_matches.clear();
                        return Some(char_match_range);
                    } else {
                        // Match isn't complete yet, move on to the next.
                        i += 1;
                    }
                } else {
                    // Doesn't match, remove it.
                    let _ = self.possible_matches.swap_remove(i);
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod code_rope_search_iterator_simple_tests {
    use super::SearchIter;

    #[test]
    fn test_search_iterator() {
        let rope = ropey::Rope::from_str("// This is a sample string with some occurrences of '//'.");
        let substring = "//";
        let slice = &rope.slice(0..);

        let search_iter = SearchIter::from_rope_slice(slice, substring);
        search_iter.for_each(|r| println!("search {:?}", r))
    }
}

struct EarlyTerminationSearch<'a> {
    char_iter: ropey::iter::Chars<'a>,
    search_pattern: &'a str,
    search_pattern_chars: Vec<char>,
    cur_index: usize, // The current char index of the search head.
    possible_match: Vec<char>, // Tracks where we are in the search pattern for the current possible match.
    match_start_index: usize, // The starting index of the current possible match.
    found_match: bool, // Flag indicating whether a match has been found.
}

impl<'a> EarlyTerminationSearch<'a> {
    fn from_rope_slice(slice: &'a RopeSlice, search_pattern: &'a str) -> EarlyTerminationSearch<'a> {
        assert!(
            !search_pattern.is_empty(),
            "Can't search using an empty search pattern."
        );
        let search_pattern_chars: Vec<char> = search_pattern.chars().collect();
        EarlyTerminationSearch {
            char_iter: slice.chars(),
            search_pattern,
            search_pattern_chars,
            cur_index: 0,
            possible_match: Vec::new(),
            match_start_index: 0,
            found_match: false,
        }
    }
}

impl<'a> Iterator for EarlyTerminationSearch<'a> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        while let Some(next_char) = self.char_iter.next() {
            self.cur_index += 1;
            if self.found_match {
                // If a match has been found, terminate early.
                return None;
            }
            if next_char == self.search_pattern_chars[self.possible_match.len()] {
                self.possible_match.push(next_char);
                if self.possible_match.len() == self.search_pattern_chars.len() {
                    // Complete match found.
                    self.found_match = true;
                    return Some((self.cur_index - self.search_pattern_chars.len(), self.cur_index));
                }
                if self.possible_match.len() == 1 {
                    // Start of a potential match.
                    self.match_start_index = self.cur_index - 1;
                }
            } else {
                // Mismatch, reset possible match.
                self.possible_match.clear();
            }
        }
        None
    }
}
#[cfg(test)]
mod code_rope_search_iterator_tests {
    use crate::code::{EarlyTerminationSearch, SearchIter};

    #[test]
    fn test_early_termination_search() {
        let rope = ropey::Rope::from_str("// This is a sample string with some occurrences of '//'.");
        let substring = "//";
        let slice = &rope.slice(0..);

        let search_iter = EarlyTerminationSearch::from_rope_slice(slice, substring);
        let result: Vec<(usize, usize)> = search_iter.collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get(0).unwrap().0, 0);
        assert_eq!(result.get(0).unwrap().1, 2);
    }


    #[test]
    fn test_full_search_bench() {
        use std::time::{Instant};

        let n = 10000000;

        let start_time = Instant::now();

        let rope = ropey::Rope::from_str("// This is a sample string with some occurrences of '//'.");
        let substring = "//";
        let slice = &rope.slice(0..);

        for _ in 0..n {
            let search_iter = SearchIter::from_rope_slice(slice, substring);
            let result: Vec<(usize, usize)> = search_iter.collect();
        }

        let elapsed_time = Instant::now().duration_since(start_time).as_secs_f64();
        let ops_per_sec = (n as f64) / elapsed_time;
        println!("ops: {:.2}", ops_per_sec);
        // ops: 204967.29
    }


    #[test]
    fn test_early_termination_search_bench() {
        use std::time::{Instant};

        let rope = ropey::Rope::from_str("This is a sample string with some occurrences of '//'.");
        let substring = "//";
        let slice = &rope.slice(0..);

        let operation = || {
            let search_iter = EarlyTerminationSearch::from_rope_slice(slice, substring);
            let result: Vec<(usize, usize)> = search_iter.collect();
        };

        let n = 10_000;
        let start_time = Instant::now();
        for _ in 0..n { operation(); }

        let elapsed_time = Instant::now().duration_since(start_time).as_secs_f64();
        let ops_per_sec = (n as f64) / elapsed_time;
        println!("ops: {:.2}", ops_per_sec);
        // ops: 211009.97
    }
}

#[cfg(test)]
mod code_rope_search_tests {
    use ropey::RopeSlice;

    fn rope_search(rope: &RopeSlice, substring: &str) -> Option<usize> {
        let sub_len = substring.len();

        for i in 0..=rope.len_chars() - sub_len {
            let sb = rope.slice(i..i + sub_len).as_str();
            if sb.is_some() && sb.unwrap() == substring {
                return Some(i);
            }
        }
        None
    }

    #[test]
    fn test_search() {
        let rope = ropey::Rope::from_str("This is a sample string with some occurrences of '//'.");
        let substring = "//";
        let slice = &rope.slice(0..);

        let result = rope_search(slice, substring);
        match result {
            Some(r) => println!("result {}", r),
            None => println!("not found"),
        }
    }


    #[test]
    fn test_rope_search_bench() {
        use std::time::{Instant};

        let rope = ropey::Rope::from_str("This is a sample string with some occurrences of '//'.");
        let substring = "//";
        let slice = &rope.slice(0..);

        let operation = || {
            let result = rope_search(slice, substring);
        };

        let n = 10_000;
        let start_time = Instant::now();
        for _ in 0..n { operation(); }

        let elapsed_time = Instant::now().duration_since(start_time).as_secs_f64();
        let ops_per_sec = (n as f64) / elapsed_time;
        println!("ops: {:.2}", ops_per_sec);
        // ops: 25167.75
        // slow
    }
}

// Enum to represent different types of operations
#[derive(Debug, Clone)]
pub enum Operation {
    Insert,
    Remove,
    Start,
    End
}

// Change struct to represent a single change operation
#[derive(Debug, Clone)]
pub struct Change {
    pub start: usize,
    pub operation: Operation,
    pub text: String,
    pub row: usize,
    pub column: usize
}

#[derive(Debug, Default)]
pub struct MultipleChange {
    pub changes: Vec<Change>,
}

impl Code {
    pub fn undo(&mut self) -> Option<MultipleChange> {
        let mut multiple_change = MultipleChange::default();
        let mut end = false;
        let mut multiple = false;

        while !end {
            match self.undo_history.pop() {
                None => return None,
                Some(change) => {
                    match change.operation {
                        Operation::Insert => {
                            let from = change.start;
                            let to = from + change.text.chars().count();
                            self.remove(from, to);
                            multiple_change.changes.push(change.clone());
                            self.redo_history.push(change);
                            if !multiple { return Some(multiple_change) }
                        },
                        Operation::Remove => {
                            self.insert(&change.text, change.start);
                            multiple_change.changes.push(change.clone());
                            self.redo_history.push(change);
                            if !multiple { return Some(multiple_change) }
                        },
                        Operation::End => multiple = true,
                        Operation::Start => end = true,
                    }
                }
            }
        }

        Some(multiple_change)
    }

    pub fn redo(&mut self) -> Option<MultipleChange> {
        let mut multiple_change = MultipleChange::default();
        let mut end = false;
        let mut multiple = false;

        while !end {
            match self.redo_history.pop() {
                None => return None,
                Some(change) => {
                    match change.operation {
                        Operation::Insert => {
                            self.insert(&change.text, change.start);
                            multiple_change.changes.push(change.clone());
                            self.undo_history.push(change);
                            if !multiple { return Some(multiple_change) }
                        },
                        Operation::Remove => {
                            let from = change.start;
                            let to = from + change.text.chars().count();
                            self.remove(from, to);
                            multiple_change.changes.push(change.clone());
                            self.undo_history.push(change);
                            if !multiple { return Some(multiple_change) }
                        }
                        Operation::End => multiple = true,
                        Operation::Start => end = true,
                    }
                }
            }
        }

        Some(multiple_change)
    }
}


#[cfg(test)]
mod code_undo_tests {
    use crate::code::Code;

    #[test]
    fn test_code_undo() {
        let mut buffer = Code::new();

        buffer.insert_text("hello", 0, 0);
        buffer.insert_text(" world", 0, 5);

        println!("{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);

        buffer.undo();

        println!("{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);

    }

    #[test]
    fn test_code_redo() {
        let mut buffer = Code::new();

        // Insert initial text
        buffer.insert_text("hello", 0, 0);
        buffer.insert_text(" world", 0, 5);
        assert_eq!(buffer.text.to_string(), "hello world");

        // Undo the last change
        buffer.undo();
        assert_eq!(buffer.text.to_string(), "hello");

        // Redo the change
        buffer.redo();
        assert_eq!(buffer.text.to_string(), "hello world");

        // Test multiple operations
        buffer.insert_text("!", 0, 11);
        assert_eq!(buffer.text.to_string(), "hello world!");

        // Undo multiple times
        buffer.undo();
        assert_eq!(buffer.text.to_string(), "hello world");
        buffer.undo();
        assert_eq!(buffer.text.to_string(), "hello");
        buffer.undo();
        assert_eq!(buffer.text.to_string(), "");

        // Redo multiple times
        buffer.redo();
        assert_eq!(buffer.text.to_string(), "hello");
        buffer.redo();
        assert_eq!(buffer.text.to_string(), "hello world");
        buffer.redo();
        assert_eq!(buffer.text.to_string(), "hello world!");
    }
}


impl Code {
    pub fn move_line_down(&mut self, line_idx: usize) -> bool {
        let len_lines = self.text.len_lines();
        if len_lines <= 2 { return false; }

        let line1_start = match self.text.try_line_to_char(line_idx).ok(){
            Some(idx) => idx, None => return false,
        };
        let line1_end = match self.text.try_line_to_char(line_idx + 1).ok(){
            Some(idx) => idx-1, None => return false,
        };
        let line2_start = match self.text.try_line_to_char(line_idx + 1).ok(){
            Some(idx) => idx, None => return false,
        };
        let line2_end = match self.text.try_line_to_char(line_idx + 2).ok(){
            Some(idx) if idx == self.text.len_chars() => idx,
            Some(idx) => idx-1,
            None => return false,
        };

        // if line1_end == self.text.len_chars() { return false; } // skip last line

        let line_1 = self.text.slice(line1_start..line1_end).to_string();
        let line_2 = self.text.slice(line2_start..line2_end).to_string();
        // let text = self.get_text(line_idx, 0, line_idx+1, 0);

        self.undo_history.push(Change {
            start: 0, operation: Operation::Start,
            text: "".to_string(), row:0, column:0
        });

        self.remove_text(line_idx, 0, line_idx, line_1.chars().count());
        self.insert_text(&line_2, line_idx, 0);
        self.remove_text(line_idx+1, 0, line_idx+1, line_2.chars().count());
        self.insert_text(&line_1, line_idx+1, 0);

        self.undo_history.push(Change {
            start: 0, operation: Operation::End,
            text: "".to_string(), row:0, column:0
        });

        return true;
    }

    pub fn move_line_up(&mut self, line_idx: usize) -> bool {
        self.move_line_down(line_idx-1)
    }
}

#[cfg(test)]
mod code_move_line_test {
    use crate::code::Code;

    #[test]
    fn test_code_move_line_down() {
        let mut buffer = Code::from_str("hello\nworld\na");

        println!("{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);

        buffer.move_line_down(0);

        println!("\n-------move hello to world-------------");
        println!("{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);

        assert_eq!(buffer.text.to_string(), "world\nhello\na");

        buffer.undo();

        println!("\n--------------------\n{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);
        assert_eq!(buffer.text.to_string(), "hello\nworld\na");
    }

    #[test]
    fn test_code_move_line_up() {
        let mut buffer = Code::new();
        buffer.insert_text("hello\nworld\na", 0, 0);

        println!("{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);

        buffer.move_line_up(1);

        println!("\n--------------------\n{}", buffer.text.to_string());
        println!("{:?}", buffer.undo_history);

        assert_eq!(buffer.text.to_string(), "world\nhello\na");
    }

    #[test]
    fn test_code_move_line_down_last_line() {
        let mut buffer = Code::from_str("1\n2\n3\n4");
        println!("{}", buffer.text.to_string());

        buffer.move_line_down(2);
        println!("\n-------move 3 to 4-------------");

        println!("{}", buffer.text.to_string());

        assert_eq!(buffer.text.to_string(), "1\n2\n4\n3");
    }
}

#[cfg(test)]
mod code_indentation_tests {
    use crate::code::Code;

    #[test]
    fn test_code_indentation_level() {
        let config = crate::config::get();
        let mut code = Code::from_str("    print('hello')");
        code.set_lang("python".to_string(), &config);

        println!("{}", code.text.to_string());

        let il = code.indentation_level(0);
        println!("indentation_level on line is {il}");

        assert_eq!(il, 1);
    }

    #[test]
    fn test_code_indentation_level_2() {
        let config = crate::config::get();
        let mut code = Code::from_str("        print('hello')");
        code.set_lang("python".to_string(), &config);

        println!("{}", code.text.to_string());

        let il = code.indentation_level(0);
        println!("indentation_level on line is {il}");

        assert_eq!(il, 2);
    }

    #[test]
    fn test_code_indentation_only() {
        let config = crate::config::get();
        let mut code = Code::from_str("        print('hello')");
        code.set_lang("python".to_string(), &config);

        println!("{}", code.text.to_string());

        let il = code.is_only_indentation_before(0,8);
        println!("indentation_level on line is {il}");

        assert_eq!(il, true);
    }
}
