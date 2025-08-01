use anyhow::{Result};
use std::path::{Path, PathBuf};
use std::{fs, time};
use rayon::prelude::*;
use crate::utils::IGNORE_EXTS;

#[derive(Debug)]
pub struct Search {
    pub active: bool,
    pub pattern: ropey::Rope,
    pub results: Vec<SearchResult>,
    pub index: usize,
    pub cursor_pos: usize,
}

#[derive(Debug)]
pub struct SearchResult {
    pub line: usize,
    pub column: usize,
    pub preview: Option<String>,
}

impl Search {
    pub fn new() -> Self {
        Self {
            active: false,
            pattern: ropey::Rope::new(),
            results: Vec::new(),
            index: 0,
            cursor_pos: 0,
        }
    }
}

#[derive(Debug)]
pub struct FileSearchResult {
    pub file_path: String,
    pub search_results: Vec<SearchResult>,
}

pub fn search_in_directory(
    directory_path: &std::path::Path,
    substring_to_find: &str,
) -> Result<Vec<FileSearchResult>> {
    use rayon::prelude::*;

    let file_paths = read_directory_recursive(directory_path)?;

    let results = file_paths
        .par_iter()
        .filter_map(|file_path| {
            let path = file_path.to_str().expect("Invalid file path");
            let search_results = search_on_file(path, substring_to_find).ok()?;

            if !search_results.is_empty() {
                Some(FileSearchResult {
                    file_path: file_path.to_string_lossy().to_string(),
                    search_results,
                })
            } else {
                None
            }
        })
        .collect();
    
    Ok(results)
}

pub fn read_directory_recursive(
    dir_path: &std::path::Path
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    
    let mut paths = Vec::new();
    
    let entries = match std::fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    // Convert entries to Vec to enable parallel processing
    let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    
    // Process entries in parallel
    let mut sub_paths: Vec<std::path::PathBuf> = entries.par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_string_lossy().to_lowercase();

            if file_name.starts_with('.') || crate::utils::IGNORE_DIRS.contains(&file_name.as_str()) {
                return None;
            }

            if path.is_dir() {
                match read_directory_recursive(&path) {
                    Ok(sub_paths) => Some(sub_paths),
                    Err(_) => None,
                }
            } else {
                let file_ext = path.extension()?.to_string_lossy().to_lowercase();
                if !IGNORE_EXTS.contains(&file_ext.as_str()) {
                    Some(vec![path])
                } else {
                    None
                }
            }
        })
        .flatten()
        .collect();

    paths.append(&mut sub_paths);
    Ok(paths)
}

fn search_on_file(
    file_path: &str, substring_to_find: &str
) -> Result<Vec<SearchResult>> {
    use std::io::prelude::*;

    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);

    let mut results = Vec::new();
    let mut line_number = 0;

    for line_result in reader.lines() {
        line_number += 1;
        let line = line_result?;
        let mut search_start = 0;

        while let Some(byte_index) = line[search_start..].find(substring_to_find) {
            // count symbols before byte index
            let symbol_column = line[..search_start + byte_index].chars().count();

            results.push(SearchResult {
                line: line_number,
                column: symbol_column,
                preview: Some(line.clone()),
            });

            // move start next
            search_start += byte_index + substring_to_find.len();
        }
    }

    Ok(results)
}