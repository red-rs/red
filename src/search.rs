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

    pub const IGNORE_EXTS: &[&str] = &[
        "doc", "docx", "pdf", "rtf", "odt", "xlsx", "pptx", "jpg", "png", "gif", "bmp", "svg",
        "tiff", "mp3", "wav", "aac", "flac", "ogg", "mp4", "avi", "mov", "wmv", "mkv", "zip",
        "rar", "tar.gz", "7z", "exe", "msi", "bat", "sh", "so", "ttf", "otf",
    ];

    pub fn read_directory_recursive(dir_path: &Path) -> Result<Vec<PathBuf>, io::Error> {
        let mut paths = Vec::new();

        let entries = fs::read_dir(dir_path)?;

        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let file_name = match path.file_name() {
                Some(name) => name.to_string_lossy().to_lowercase(),
                None => continue,
            };

            if crate::utils::IGNORE_DIRS.contains(&file_name.to_string().as_str()) {
                continue;
            }

            if path.is_dir() {
                let sub_dir_result = read_directory_recursive(&path)?;
                paths.extend(sub_dir_result);
            } else {
                let file_ext = match path.extension() {
                    Some(ext) => ext.to_string_lossy().to_lowercase(),
                    None => continue,
                };

                if !IGNORE_EXTS.contains(&file_ext.to_string().as_str()) {
                    paths.push(path);
                }
            }
        }

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

        let file_paths = read_directory_recursive(directory_path)?;

        file_paths
            // .into_iter()
            .par_iter()
            .map(|file_path| {
                let path = file_path.to_str().expect("Invalid file path");
                let search_results = search_on_file(path, substring_to_find)?;
                Ok(FileSearchResult {
                    file_path: file_path.to_string_lossy().to_string(),
                    search_results,
                })
            })
            .collect()
    }

    use tokio::task;

    #[tokio::test]
    async fn test_find_substring_on_dir() {
        use std::path::Path;

        // let directory_path = Path::new("/Users/max/Downloads/spark");
        let directory_path = Path::new("./");
        let substring_to_find = "test";

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
            Err(_) => {}
        }
    }

    // async fn process_file(
    //     file_path: PathBuf,
    //     substring_to_find: &str,
    // ) -> io::Result<FileSearchResult> {
    //     let path = file_path.to_str().expect("Invalid file path");
    //     let search_results = search_on_file(path, substring_to_find)?;
    //     Ok(FileSearchResult {
    //         file_path,
    //         search_results,
    //     })
    // }

    //
    // async fn search_in_directory_par(directory_path: &Path, substring_to_find: &str)
    //                                  -> io::Result<Vec<FileSearchResult>> {
    //
    //     let file_paths = read_directory_recursive(directory_path)?;
    //
    //     let mut tasks = Vec::new();
    //
    //     for file_path in file_paths {
    //
    //         let substring = substring_to_find.to_string(); // Clone the string
    //
    //         let task = tokio::task::spawn_blocking( move || {
    //             process_file(file_path.clone(), &substring)
    //         });
    //
    //         tasks.push(task);
    //     }
    //
    //     for task in tasks {
    //         let x = task.await.unwrap();
    //
    //     }
    //
    //     Ok(vec![])
    // }
    //
    // #[tokio::test]
    // async fn my_test() {
    //     let directory_path = Path::new("./");
    //     let substring_to_find = "test";
    //     let results = search_in_directory_par(directory_path, substring_to_find).await.unwrap();
    //
    //     // Now you can assert or perform actions on the results
    //     assert_eq!(results.len(), 4);
    //     // Add more assertions as needed
    // }
    // doesnt work!
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
