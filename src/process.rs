use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub struct Process {
    kill_sender: Option<tokio::sync::mpsc::Sender<String>>,
    process_lines: Arc<Mutex<Vec<String>>>,
    upd_process: Arc<AtomicBool>,
    process_started: Arc<Mutex<bool>>,
    last_cmd: String
}

impl Process {
    pub fn new() -> Self {
        Self {
            kill_sender: None,
            process_lines: Arc::new(Mutex::new(vec![])),
            upd_process: Arc::new(AtomicBool::new(false)),
            process_started: Arc::new(Mutex::new(false)),
            last_cmd: String::new(),
        }
    }
   
    pub fn run_tmux(&mut self, args:&String) {
        let red_home = std::env::var("RED_HOME").expect("RED_HOME must be set");
        let tmux_path = std::path::Path::new(&red_home).join("tmux.sh");

        let cmd = match tmux_path.to_str() {
            Some(tmux) => tmux.to_string(),
            None => return,
        };

        let args = vec![args.clone()];
        self.last_cmd = args.join(" ");

        tokio::spawn(async move {
            Command::new(&cmd).args(args)
                .output().await.unwrap();
        });
    }

    pub fn run_last_tmux(&mut self) {
        if self.last_cmd.is_empty() { return }
        let last_cmd = self.last_cmd.clone();
        self.run_tmux(&last_cmd);
    }

    pub fn start(&mut self, cmd: &str, arg: &str) {
        let mut is_started = self.process_started.lock().expect("cant get lock");
        if *is_started {
            return;
        }

        let mut child = tokio::process::Command::new(cmd)
            .arg(arg)
            .env("PYTHONUNBUFFERED", "false")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("cant spawn cmd");

        let (send, mut child_stdout_receiver) = tokio::sync::mpsc::channel::<String>(10);
        let (kill_send, mut kill) = tokio::sync::mpsc::channel::<String>(10);

        let child_stdout = child.stdout.take().expect("can not get stdout");

        self.kill_sender = Some(kill_send);

        let mut lines = self.process_lines.lock().unwrap();
        lines.clear();

        *is_started = true;

        // prepare data for reading stdout task
        let process_lines_data = self.process_lines.clone();
        let upd_process_needed = self.upd_process.clone();

        tokio::spawn(async move {
            // reading stdout task
            let reader = BufReader::new(child_stdout);
            let mut lines = reader.lines();

            while let Some(line) = lines.next_line().await.expect("can not read message") {
                let mut lines = process_lines_data.lock().unwrap();
                (*lines).push(line.clone());
                upd_process_needed.store(true, Ordering::SeqCst)
            }
        });

        // prepare data for kill task
        let process_lines_data = self.process_lines.clone();
        let process_started = self.process_started.clone();
        let upd_process_needed = self.upd_process.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(m) = kill.recv() => { // killing manually
                        child.kill().await.expect("kill failed");

                        let mut is_started = process_started.lock().expect("cant get lock");
                        *is_started = false;

                        let mut lines = process_lines_data.lock().unwrap();
                        (*lines).push("Killed".to_string());

                        upd_process_needed.store(true, Ordering::SeqCst);
                        break;
                    }
                    _ = child.wait() => { // process ends
                        let mut is_started = process_started.lock().expect("cant get lock");
                        *is_started = false;

                        let mut lines = process_lines_data.lock().unwrap();
                        (*lines).push("Process ended".to_string());

                        upd_process_needed.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            }
        });
    }

    pub fn update_true(&self) {
        self.upd_process.store(true, Ordering::SeqCst);
    }
    pub fn update_false(&self) {
        self.upd_process.store(false, Ordering::SeqCst)
    }

    pub fn kill_process(&mut self) {
        let process_started = self.process_started.lock().expect("cant get lock");
        if *process_started == false {
            return;
        }

        if let Some(sender) = self.kill_sender.take() {
            tokio::spawn(async move {
                sender
                    .send("".to_owned())
                    .await
                    .expect("can not send message")
            });
        }
    }

    pub fn upd(&self) -> bool {
        self.upd_process.load(Ordering::Acquire)
    }

    pub fn lines(&self) -> Arc<Mutex<Vec<String>>> {
        self.process_lines.clone()
    }

    fn lines_range(&self, start_index: usize, end_index: usize) -> Option<Vec<String>> {
        let lines = self.process_lines.lock().unwrap(); // Lock the Mutex to access the vector
        if start_index < end_index && end_index < lines.len() {
            Some(lines[start_index..end_index].to_vec()) // Extract elements from start_index to end
        } else {
            None // Return None if start_index is out of bounds
        }
    }
}


mod process_tests {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use super::Process;

    // #[tokio::test]
    // async fn test_process_start() {
    //     let mut process = Process::new();
    //     process.start("echo", "Hello, World!");

    //     // Wait for some time to allow process to start and output something
    //     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    //     let lines_lock = process.lines().lock();
    //     let lines = lines_lock.unwrap();
    //     assert!(lines.len() > 0);
    //     assert_eq!(lines[0], "Hello, World!");
    // }

    // #[tokio::test]
    // async fn test_process_kill() {
    //     let mut process = Process::new();
    //     process.start("sleep", "10"); // A long-running process

    //     tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    //     process.kill_process().await;

    //     // Wait for some time to allow the process to be killed
    //     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    //     let lines = process.lines().lock().await;
    //     assert!(lines.contains(&"Killed".to_string()));
    // }

    // #[tokio::test]
    // async fn test_process_update() {
    //     let process = Process::new();
    //     process.start("echo", "Hello, World!");

    //     // Wait for some time to allow process to start and output something
    //     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    //     assert!(process.upd());

    //     process.update_false();
    //     assert!(!process.upd());
    // }

    #[tokio::test]
    async fn test_lines_range() {
        let mut process = Process::new();
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        process.start("echo", "Line 1");
        process.start("echo", "Line 2");

        // Wait for some time to allow process to output lines
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let lines = process.lines_range(0, 2);
        // assert!(lines.is_some());
        let lines = lines.unwrap();
        assert_eq!(lines[0], "Line 1");
        assert_eq!(lines[1], "Line 2");
    }

}