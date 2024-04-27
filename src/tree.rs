use std::f32::consts::E;
//tree.rs
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::io::{self, Write};
use crossterm::{cursor, queue, QueueableCommand, style::Print};
use log2::debug;
use serde::de;
use tokio::sync::watch::error;

use crate::utils;
use crate::utils::{IGNORE_DIRS, IGNORE_FILES};
use crossterm::style::{Color, SetBackgroundColor as BColor, SetForegroundColor as FColor};

#[derive(Debug)]
pub struct TreeNode {
    name: String,
    fullpath: String,
    is_file: bool,
    children: Option<Vec<TreeNode>>,
}

impl TreeNode {
    pub fn new(name:String, fullpath:String, is_file: bool) -> Self {
        Self { name, fullpath, is_file, children: None }
    }
    pub fn print(&self) { println!("node {:?}", self); }
    pub fn is_file(&mut self) -> bool { self.is_file }
    pub fn fullpath(&mut self) -> String { self.fullpath.clone() }
    pub fn collapse(&mut self) { self.children = None; }

    pub fn expand(&mut self) -> io::Result<()> {
        if !Path::new(&self.fullpath).is_dir() { return Ok(()); }

        let mut children = Vec::new();
       
        let mut directories = Vec::new();
        let mut files = Vec::new();

        for entry in fs::read_dir(&self.fullpath)? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap();
            let is_file = entry.file_type().unwrap().is_file();
            let abs_path = match entry.path().canonicalize() {
                Ok(abs) => abs,
                Err(e) => { debug!("cant get abs_path for {}", name); continue;},
            };
            let fullpath = abs_path.to_string_lossy().to_string();

            if !is_file && IGNORE_DIRS.contains(&name.as_str()) { continue; }
            if is_file && IGNORE_FILES.contains(&name.as_str()) { continue; }

            if is_file { files.push(TreeNode::new(name, fullpath, is_file)); }
            else { directories.push(TreeNode::new(name, fullpath, is_file)); }
        }

        directories.sort_by(|a, b| a.name.cmp(&b.name));
        files.sort_by(|a, b| a.name.cmp(&b.name));

        children.extend(directories);
        children.extend(files);

        self.children = Some(children);
        Ok(())
    }

    pub fn toggle(&mut self) -> io::Result<()> {
        if self.children.is_none() {
            self.expand()?;
        } else {
            self.collapse();
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        match &self.children {
            Some(children) => {
                let mut count = children.len();
                for child in children {
                    count += child.len();
                }
                count
            }
            None => 0,
        }
    }

    fn filter_files_mutate(&mut self, pattern: &str) -> bool {
        let mut found = false;
        if let Some(children) = &mut self.children {
            let mut filtered_children = Vec::new();
            for mut child in children.drain(..) {
                if child.is_file && child.name.contains(pattern) {
                    found = true;
                    filtered_children.push(child);
                } else if !child.is_file {
                    child.expand();
                    // Recursive call for directories
                    let is_any_found = child.filter_files_mutate(pattern);
                    if is_any_found {
                        filtered_children.push(child);
                        found = true;
                    }
                }
            }
            self.children = Some(filtered_children);
        }
        found
    }
}

pub struct TreeNodeIterator<'a> {
    stack: Vec<(&'a TreeNode, usize)>, // Tuple of node reference and depth
}

impl<'a> TreeNodeIterator<'a> {
    pub fn new(root: &'a TreeNode) -> Self {
        let mut stack = Vec::new();
        stack.push((root, 0)); // Start with depth 0 for the root node
        Self { stack }
    }
}

impl<'a> Iterator for TreeNodeIterator<'a> {
    type Item = (&'a TreeNode, usize); // Tuple of node reference and depth

    fn next(&mut self) -> Option<Self::Item> {
        // Pop the last node-depth tuple from the stack
        let (node, depth) = self.stack.pop()?;

        // Push children onto the stack with incremented depth
        if let Some(children) = &node.children {
            let new_depth = depth + 1;
            for child in children.iter().rev() {
                self.stack.push((child, new_depth));
            }
        }

        Some((node, depth))
    }
}

pub struct TreeView {
    width: usize,
    height: usize,
    dir: String,
    pub upd: bool,
    root: TreeNode,

    selected: usize,
    x: usize,
    moving: bool,
    /// Color for tree dir.
    dir_color: Color,
    /// Color for tree dir.
    file_color: Color,

    active_file: String,
    /// Color for active file.
    active_file_color: Color,

    search: FileSearch,
}

impl TreeView {
    pub fn new(dir:String) -> Self {
        let name = if dir == "."  || dir == "./" {
            utils::current_directory_name().unwrap() 
        } else { dir.to_string() };

        let mut root = TreeNode {
            name,
            fullpath: utils::abs_file(&dir), 
            is_file: false,
            children: None,

        };

        root.expand();

        Self { width: 25, height: 30, dir, upd: true, root, selected:0, x: 0,
            moving: false, dir_color: Color::Reset, file_color: Color::Reset,
            active_file: String::new(), active_file_color: Color::Reset,
            search: FileSearch::new(),
        }
    }

    pub fn set_width(&mut self, width: usize) { self.width = width; self.upd = true; }
    pub fn set_height(&mut self, height: usize) { self.height = height; self.upd = true; }
    pub fn set_dir_color(&mut self, c: Color) { self.dir_color = c; self.upd = true; }
    pub fn set_file_color(&mut self, c: Color) { self.file_color = c; self.upd = true; }
    pub fn set_active_file_color(&mut self, c: Color) { self.active_file_color = c; self.upd = true; }
    pub fn set_moving(&mut self, m: bool) { self.moving = m; self.upd = true; }
    pub fn set_selected(&mut self, i: usize) { self.selected = i + self.x; self.upd = true; }
    pub fn is_moving(&mut self) -> bool { self.moving }
    pub fn is_search(&mut self) -> bool { self.search.active }

    pub(crate) fn handle_up(&mut self) {
        if self.selected == 0 { return; }
        self.selected -= 1;
        self.upd = true;
    }
    pub(crate) fn handle_down(&mut self) {
        if self.selected >= self.root.len() { 
            return; 
        }
        self.selected += 1;
        self.upd = true;
    }

    pub fn scroll_down(&mut self) {
        if self.x + self.height > self.root.len() { 
            return; 
        }

        self.x += 1;
        self.upd = true;
    }
    pub fn scroll_up(&mut self) {
        if self.x == 0 { return; }

        self.x -= 1;
        self.upd = true;
    }

    pub fn expand_root(&mut self) {
        let root = &mut self.root;
        root.expand();

    }

    pub fn filter_files_by_pattern(&mut self, pattern: &str) {
        let root = &mut self.root;
        root.expand();
        root.filter_files_mutate(pattern);

        let mut index = 0;
        Self::find_first_file_index(root, &mut index);
        self.selected = index;
    }


    pub fn draw(&mut self) {
        if !self.upd { return; }
        if self.width == 0 { return; }

        let mut stdout = std::io::stdout();

        let padding_left = 1;

        let iter = TreeNodeIterator::new(&self.root);
        let iter = iter.skip(self.x).take(self.height);
        let mut count = 0;

        queue!(stdout, cursor::Hide);

        for (i, (node, depth)) in iter.enumerate() {
            // if i > self.height { break; }

            queue!(stdout, cursor::MoveTo(0, i as u16));

            let mut col = 0; 

            let mut color = if node.is_file { 
                if node.fullpath.eq(&self.active_file) { self.active_file_color } 
                else { self.file_color }
            } else { self.dir_color };

            if self.selected == i+ self.x { color = self.active_file_color }

            for i in 0..padding_left {
                if col >= self.width-1 { break; }
                queue!(stdout, Print(' '));
                col += 1;
            }
            for i in 0..depth {
                if col >= self.width-1 { break; }
                queue!(stdout, Print(' '));
                col += 1;
            }
            for ch in node.name.chars().take(self.width-padding_left-depth-1) {
                if col >= self.width-1 { break; }
                queue!(stdout, FColor(color), Print(ch));
                col += 1;
            }
            
            if col < self.width {
                for i in 0..self.width-col-1 {
                    queue!(stdout, Print(' '));
                }
            }
            queue!(stdout, FColor(Color::DarkGrey), Print('│'));

            count += 1;
        }

        while count < self.height { // fill empty space
            queue!(stdout, cursor::MoveTo(0, count as u16));
            queue!(stdout, Print(" ".repeat(self.width-1)));
            queue!(stdout, FColor(Color::DarkGrey), Print('│'));
            count += 1;
        }

        self.draw_search();

        self.upd = false;
    }

    pub fn draw_search(&self) {
        if !self.search.active || self.width == 0 { return }

        let mut stdout = std::io::stdout();
        let prefix = " search: ";
        let search = format!("{}{}", prefix, self.search.pattern.to_string());
        if search.len() >= self.width { return; } // not enought space
        queue!(stdout,cursor::Show, cursor::MoveTo(0, (self.height -1) as u16));
        queue!(stdout, Print(&search));
        queue!(stdout, Print(" ".repeat(self.width-search.len()-1)));
        queue!(stdout, FColor(Color::DarkGrey), Print('│'));
        queue!(stdout, cursor::MoveTo((prefix.len() + self.search.index) as u16, (self.height -1) as u16));
        // stdout.flush();
    }
    pub fn print(&self) {
        self.print_node(&self.root, 0, &mut 0);
    }

    fn print_node(&self, node: &TreeNode, depth: usize, count: &mut usize) {
        println!("{}{}: {}", " ".repeat(depth), count, node.name);

        // Recursively print children
        if let Some(children) = &node.children {
            for child in children {
                *count += 1;
                self.print_node(child, depth + 1, count);
            }
        }
    }


    pub fn find<'a>(&'a mut self, index: usize) -> Option<&'a mut TreeNode> {
        let mut count = 0;
        let root = &mut self.root;
        let maybe_node = Self::find_by_index(root, index + self.x, &mut count);
        maybe_node
    }
    
    pub fn get_selected<'a>(&'a mut self) -> Option<&'a mut TreeNode> {
        let mut count = 0;
        let root = &mut self.root;
        let maybe_node = Self::find_by_index(root, self.selected, &mut count);
        maybe_node
    }

    pub fn find_and_expand(&mut self, index: usize) {
        let mut count = 0;
        let root = &mut self.root;
        let maybe_node = Self::find_by_index(root, index, &mut count);
        maybe_node.map(|node| node.expand());
    }

    pub fn find_expand_by_fullpath(&mut self, fullpath: &str) {
        let root = &mut self.root;
        Self::find_by_fullpath_and_expand(root, fullpath);
    }

    pub fn find_and_toggle(&mut self, index: usize) {
        let mut count = 0;
        let root = &mut self.root;
        let maybe_node = Self::find_by_index(root, index, &mut count);
        maybe_node.map(|node| node.toggle());
    }

    fn find_by_index<'a>(node: &'a mut TreeNode, index: usize, count: &mut usize) -> Option<&'a mut TreeNode>{
        if *count == index {
            // println!("Found {}: {}", index, node.name);
            // node.expand();
            return Some(node);
        }

        // Recursively search children
        if let Some(children) = &mut node.children {
            for child in children {
                *count += 1;
                let found_child = Self::find_by_index(child, index, count);
                if found_child.is_some() { return found_child;}
            }
        }
        None
    }
    
    fn find_by_index_expand(node: &mut TreeNode, index: usize, count: &mut usize) -> bool {
        if *count == index {
            // println!("Found {}: {}", index, node.name);
            node.expand();
            return true;
        }

        // Recursively search children
        if let Some(children) = &mut node.children {
            for child in children {
                *count += 1;
                let found = Self::find_by_index_expand(child, index, count);
                if found { return true; }
            }
        }
        return false;
    }

    fn find_first_file_index(node: &mut TreeNode, index: &mut usize) -> bool {
        if node.is_file {
            // println!("Found {}: {}", node.name, index);
            node.expand();
            return true;
        }

        // Recursively search children
        if let Some(children) = &mut node.children {
            for child in children {
                *index += 1;
                let found = Self::find_first_file_index(child, index);
                if found { return true; }
            }
        }
        return false;
    }

    pub fn find_by_fullpath_and_expand(node: &mut TreeNode, fullpath: &str) -> bool {
        if fullpath.starts_with(&node.fullpath) {
            node.expand();
        }
        // Recursively search children
        if let Some(children) = &mut node.children {
            for child in children {
                if fullpath.starts_with(&child.fullpath) {
                    child.expand();
                    // return true;
                }
                let found = Self::find_by_fullpath_and_expand(child, fullpath);
                if found {
                    // node.expand();
                    return true;
                }
            }
        }
        return false;
    }

    pub fn set_active(&mut self, fullpath: String) {
        self.active_file = fullpath;
        // todo: expand all nodes
    }
    
    pub async fn handle_mouse(&mut self, e: crossterm::event::MouseEvent) {
       match e {
            crossterm::event::MouseEvent { row, column, kind, modifiers } =>  {
                match kind {
                    crossterm::event::MouseEventKind::ScrollUp => self.scroll_up(),
                    crossterm::event::MouseEventKind::ScrollDown => self.scroll_down(),
                    crossterm::event::MouseEventKind::Down(button) => {}
                    _ => {}
                        
                }
            }
            _ => {}
       }
    }

    pub fn insert_filter_char(&mut self, c: char) {
        self.search.active = true;
        self.search.pattern.insert_char(self.search.index, c);
        self.search.index += 1;

        self.filter_files_by_pattern(&self.search.pattern.to_string());
        self.upd = true;
    }

    pub fn remove_filter_char(&mut self) {
        if self.search.index > 0 {
            self.search.index -= 1;
            let x = self.search.index;
            self.search.active = true;
            self.search.pattern.remove(x..x+1);

            let pattern = self.search.pattern.to_string();
            if pattern.is_empty() {
                self.expand_root();
            } else {
                self.filter_files_by_pattern(&pattern);
            }

            self.upd = true;
        }
    }

    pub fn handle_left(&mut self) {
        if self.search.index > 0 {
            self.search.index -= 1;
            self.upd = true;
        };
    }

    pub fn handle_right(&mut self) {
        if self.search.index < self.search.pattern.len_chars() {
            self.search.index += 1;
            self.upd = true;
        };
    }
    pub fn clear_search(&mut self) {
        self.search = FileSearch::new();
        self.upd = true;
        self.expand_root();
    }
}

fn list_files_and_directories(path: &str) -> io::Result<Vec<String>> {
    let entries = fs::read_dir(path)?;
    let mut names = Vec::new();

    for entry in entries {
        let file_name = entry?.file_name().into_string().unwrap();
        names.push(file_name);
    }

    Ok(names)
}

#[cfg(test)]
mod tree_tests {
    use super::{list_files_and_directories, TreeNode, TreeNodeIterator};
    use crate::tree::TreeView;

    #[test]
    fn test_list_files_and_directories() {
        match list_files_and_directories(".") {
            Ok(names) => {
                for name in names {
                    println!("{}", name);
                }
            }
            Err(err) => eprintln!("Error: {}", err),
        }
    }

    #[test]
    fn test_load() {
        let root = ".".to_string();
        let mut tree = TreeView::new(root);

        tree.expand_root();
        tree.print();

        println!("find 5");
        let maybe_node = tree.find(5);
        maybe_node.map(|node| node.print());
        
        println!("expanding 5");
        tree.find_and_expand(5);
    
        tree.print();
    }

    #[test]
    fn test_expand_search() {
        let root = ".".to_string();
        let mut tree = TreeView::new(root);

        // tree.load_root();
        // tree.print();

        println!("find rs");

        tree.filter_files_by_pattern("rs");
    
        tree.print();

        // println!("find 16");
        // let maybe_node = tree.find(16);
        // maybe_node.map(|node| node.print());
    }



    #[test]
    fn test_iter() {
        // let root_node = TreeNode {
        //     name: "Root".to_string(),
        //     fullpath: "/path/to/root".to_string(),
        //     is_file: false,
        //     children: Some(vec![
        //         TreeNode {
        //             name: "Child1".to_string(),
        //             fullpath: "/path/to/root/child1".to_string(),
        //             is_file: true,
        //             children: None,
        //         },
        //         TreeNode {
        //             name: "Child2".to_string(),
        //             fullpath: "/path/to/root/child2".to_string(),
        //             is_file: false,
        //             children: Some(vec![
        //                 TreeNode {
        //                     name: "Grandchild1".to_string(),
        //                     fullpath: "/path/to/root/child2/grandchild1".to_string(),
        //                     is_file: true,
        //                     children: None,
        //                 },
        //             ]),
        //         },
        //     ]),
        //     file_search: FileSearch {},
        // };
        //
        //
        // for (node, depth) in TreeNodeIterator::new(&root_node).take(2) {
        //     println!("take depth: {}, Name: {}, Fullpath: {}, Is File: {}", depth, node.name, node.fullpath, node.is_file);
        // }
        //
        // for (node, depth) in TreeNodeIterator::new(&root_node).skip(2) {
        //     println!("skip depth: {}, Name: {}, Fullpath: {}, Is File: {}", depth, node.name, node.fullpath, node.is_file);
        // }
    }
}


#[derive(Debug)]
pub struct FileSearch {
    pub active: bool,
    pub pattern: ropey::Rope,
    pub index:usize,
}

impl FileSearch {
    pub fn new() -> Self {
        Self {
            active: false,
            pattern: ropey::Rope::new(),
            index: 0,
        }
    }
}