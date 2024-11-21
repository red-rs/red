#[derive(Debug)]
pub struct Search {
    pub active: bool,
    pub pattern: ropey::Rope,
    pub results: Vec<SearchResult>,
    pub index:usize,
}

#[derive(Debug)]
pub struct SearchResult {
    pub line: usize,
    pub position: usize
}

impl Search {
    pub fn new() -> Self {
        Self {
            active: false,
            pattern: ropey::Rope::new(),
            results: Vec::new(),
            index: 0
        }
    }
}


pub mod search {
    use std::path::{Path, PathBuf};
    use std::{fs, time};
    use rayon::prelude::*;
    
    use log2::{debug, info, error};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub const IGNORE_EXTS: &[&str] = &[
        "doc", "docx", "pdf", "rtf", "odt", "xlsx", "pptx", "jpg", "png", "gif", "bmp", "svg",
        "tiff", "mp3", "wav", "aac", "flac", "ogg", "mp4", "avi", "mov", "wmv", "mkv", "zip",
        "rar", "tar.gz", "7z", "exe", "msi", "bat", "sh", "so", "ttf", "otf",
    ];

    pub fn read_directory_recursive(dir_path: &Path) -> Result<Vec<PathBuf>, io::Error> {
        // eprintln!("read_directory_recursive {:?}", dir_path);
        
        let mut paths = Vec::new();
        
        let entries = match fs::read_dir(dir_path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        // Convert entries to Vec to enable parallel processing
        let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        
        // Process entries in parallel
        let mut sub_paths: Vec<PathBuf> = entries.par_iter()
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

    #[cfg(test)]
    mod file_search_tests {
        use std::path::Path;
        use super::read_directory_recursive;

        #[test]
        fn test_file_search() {
            let directory_path = Path::new("./");

            let file_names = read_directory_recursive(&directory_path);

            for name in file_names.unwrap() {
                println!("{}", name.display());
            }
        }
    }

    use std::fs::File;
    use std::io::{self, BufRead};
    use std::sync::Arc;
    use std::time::Instant;

    #[derive(Debug)]
    pub struct SearchResult {
        pub line: usize,
        pub column: usize,
        preview: String,
    }

    fn search_on_file(file_path: &str, substring_to_find: &str) -> io::Result<Vec<SearchResult>> {
        let file = File::open(file_path)?;
        let reader = io::BufReader::new(file);

        let mut results = Vec::new();
        let mut line_number = 0;

        for line_result in reader.lines() {
            line_number += 1;
            let line = line_result?;

            if let Some(index) = line.find(substring_to_find) {
                let search_result = SearchResult {
                    line: line_number,
                    column: index,
                    preview: line.clone(),
                };
                results.push(search_result);
            }
        }

        Ok(results)
    }

    // #[tokio::test]
    // async fn test_find_substring() {
    //     let file_path = "./src/search.rs";
    //     let substring_to_find = "test_find_substring";

    //     match search_on_file(file_path, substring_to_find) {
    //         Ok(results) => {
    //             for result in results {
    //                 println!("{:?}", result);
    //             }
    //         }
    //         Err(..) => {
    //             panic!("Test failed");
    //         }
    //     }
    // }

    #[derive(Debug)]
    pub struct FileSearchResult {
        pub file_path: String,
        pub search_results: Vec<SearchResult>,
    }

    pub fn search_in_directory(
        directory_path: &Path,
        substring_to_find: &str,
    ) -> io::Result<Vec<FileSearchResult>> {
        use rayon::prelude::*;

        let file_paths = read_directory_recursive(directory_path)
            .expect("cant get files recursively");

        // eprintln!("[]Found {} files", file_paths.len());

        let start = Instant::now();
        let files_processed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let matches_found = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let results = file_paths
            .par_iter()
            .filter_map(|file_path| {
                let files = files_processed.fetch_add(1, Ordering::Relaxed);
                // if files % 1000 == 0 {
                //     eprintln!(
                //         "Processed {} files, found {} matches ({:?} elapsed)",
                //         files,
                //         matches_found.load(Ordering::Relaxed),
                //         start.elapsed()
                //     );
                // }

                let path = file_path.to_str().expect("Invalid file path");
                let search_results = search_on_file(path, substring_to_find)
                    .ok()?;

                matches_found.fetch_add(search_results.len(), Ordering::Relaxed);

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

        // eprintln!(
        //     "Search complete: processed {} files, found {} matches in {:?}",
        //     files_processed.load(Ordering::Relaxed),
        //     matches_found.load(Ordering::Relaxed),
        //     start.elapsed()
        // );

        Ok(results)
    }


    #[tokio::test]
    async fn test_find_substring_on_dir() {
        use std::path::Path;
        println!("hi");
        let directory_path = Path::new("./");
        let substring_to_find = "red";

        let start = Instant::now();
        let search_results = search_in_directory(directory_path, substring_to_find);
        let elapsed = Instant::now() - start;

        match search_results {
            Ok(search_results) => {
                let results_count: usize =
                    search_results.iter().map(|s| s.search_results.len()).sum();
                println!(
                    "search_in_directory done, elapsed {:?} ms",
                    elapsed.as_millis()
                );
                println!("found {:?} results", results_count);
                // for result in search_results {
                //     println!("filename {:?}", result.file_path);
                //     for sr in result.search_results {
                //         println!("{:?}", sr);
                //     }
                // }
            }
            Err(e) => {
                println!("error {}", e);
            }
        }
    }

}

mod tokio_tests {
    use std::path::{Display, Path};
    use tokio::fs::File;
    use tokio::io::{self, AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn test_async_search() -> io::Result<()> {
        let search_results = search("src/search.rs", "test").await?;
        println!("Found {:?} matches", search_results.len());
        search_results.iter().for_each(|r| println!("{:?}", r));
        Ok(())
    }

    #[derive(Debug)]
    pub struct SearchResult {
        line_number: usize,
        column: usize,
        line: String,
    }

    pub async fn search(path: &str, term: &str) -> tokio::io::Result<Vec<SearchResult>> {
        let file = File::open(path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let mut ln = 0;
        let mut result_lines = vec![];

        while let Some(line) = lines.next_line().await? {
            let found = line.find(term);
            match found {
                None => {}
                Some(f) => result_lines.push(SearchResult {
                    line_number: ln,
                    column: f,
                    line: line.clone(),
                }),
            }
            ln += 1;
        }

        Ok(result_lines)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_async() -> io::Result<()> {
        use futures::future::ready;
        use futures::stream::{StreamExt, TryStreamExt};

        let tasks = vec![
            search("src/search.rs", "test"),
            search("src/tests.rs", "test"),
            // Add more search tasks as needed
        ];

        let results: Vec<_> = futures::stream::iter(tasks)
            .buffer_unordered(50)
            .try_collect()
            .await?;

        for result in results {
            for res in result {
                println!("{:?}", res);
            }
        }

        Ok(())
    }

    use std::path::PathBuf;
    use tokio::fs;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_async_listFiles() -> io::Result<()> {
        let path = Path::new("./");

        let files =
            crate::search::search::read_directory_recursive(path).expect("can not read dir");
        println!("Len files {}", files.len());

        for f in files {
            let filename = f.to_str().unwrap();
            let search_results = search(filename, "test").await?;
            println!(
                "Found {:?} matches in file={}",
                search_results.len(),
                filename
            );
            search_results.iter().for_each(|r| println!("{:?}", r));
        }

        // let search_results = list_files_recursive("src").await?;
        // println!("Found {:?} files", search_results.len());
        // search_results.iter().for_each(|r| println!("{:?}", r));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 16)]
    async fn test_tokio_spawn_tasks() {
        use tokio::time::{sleep, Duration};

        let mut tasks = vec![];
        let n = 10;
        for id in 0..n {
            let t = tokio::spawn(async move {
                let thread = std::thread::current();
                let mut thread_name = thread.id();
                println!("Async task {} started in thread={:?}", id, thread_name);
                sleep(Duration::from_millis((n - id) * 100)).await;
                println!("Async task {} done in thread={:?}", id, thread_name);
                let result = id * id;
                (id, result)
            });

            tasks.push(t);
        }

        println!("Launched {} tasks...", tasks.len());
        for task in tasks {
            let (id, result) = task.await.expect("task failed");
            println!("Task {} completed with result: {}", id, result);
        }
        println!("Ready!");
    }

    use crate::search;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_channels() {
        let (tx, mut rx) = mpsc::channel(32);
        let tx2 = tx.clone();

        tokio::spawn(async move {
            tx.send("sending from first handle").await;
        });

        tokio::spawn(async move {
            tx2.send("sending from second handle").await;
        });

        println!("Launched tasks...",);
        while let Some(message) = rx.recv().await {
            println!("GOT = {}", message);
        }
        println!("Ready!");
    }
}

mod mpsc_test {
    use std::sync::Arc;
    use tokio::io::AsyncBufReadExt;
    use tokio::sync::{mpsc, Mutex};

    #[derive(Debug)]
    pub struct SearchResult {
        line_number: usize,
        column: usize,
        line: String,
    }

    pub async fn search(
        path: &str,
        term: &str,
        sender: mpsc::Sender<SearchResult>,
    ) -> tokio::io::Result<()> {
        let file = tokio::fs::File::open(path).await?;
        let reader = tokio::io::BufReader::new(file);
        let mut lines = reader.lines();

        let mut ln = 0;

        while let Some(line) = lines.next_line().await? {
            if let Some(column) = line.find(term) {
                let search_result = SearchResult {
                    line_number: ln,
                    column,
                    line,
                };
                // Send search result through the channel
                sender
                    .send(search_result)
                    .await
                    .expect("Channel send error");
            }
            ln += 1;
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_async() -> std::io::Result<()> {
        let (sender, mut receiver) = mpsc::channel::<SearchResult>(100);

        // Spawn search tasks
        let tasks = vec![
            search("src/search.rs", "test", sender.clone()),
            search("src/tests.rs", "test", sender.clone()),
            // Add more search tasks as needed
        ];

        for t in tasks {
            let sender = sender.clone();
            tokio::spawn(async move {
                t.await;
            });
        }

        while let Some(message) = receiver.recv().await {
            println!("GOT = {:?}", message);
        }

        Ok(())
    }

    #[cfg(test)]
    mod insert_russian_character_tests {
        #[test]
        fn test_insert_russian_character() {
            // Create a new empty String
            let mut s = String::new();

            // Insert a Russian character into the String
            s.insert_str(0, "п");
            s.insert_str(1, "р");

            println!("{}", s);
            assert_eq!(s, "пр");
        }
        
        #[test]
        fn test_insert_russian_character_to_rope() {
            // Create a new empty String
            let mut s = ropey::Rope::new();

            // Insert a Russian character into the String
            s.insert_char(0, 'п');
            s.insert_char(1, 'р');

            println!("{}", s);
            assert_eq!(s.to_string(), "пр");
        }
    }
}
