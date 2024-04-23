use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct Process {
    kill_sender: Option<tokio::sync::mpsc::Sender<String>>,
    process_lines: Arc<Mutex<Vec<String>>>,
    upd_process: Arc<AtomicBool>,
    process_started: Arc<Mutex<bool>>,
}

impl Process {
    pub fn new() -> Self {
        Self {
            kill_sender: None,
            process_lines: Arc::new(Mutex::new(vec![])),
            upd_process: Arc::new(AtomicBool::new(false)),
            process_started: Arc::new(Mutex::new(false)),
        }
    }
   
    pub fn start_tmux(&mut self, args:&String) {
        let red_home = option_env!("RED_HOME").unwrap_or("./");
        let tmux_path = std::path::Path::new(red_home).join("tmux.sh");
        let tmux = tmux_path.to_str();
        if tmux.is_none() { return; }

        let cmd = tmux.unwrap().to_string();
        let args = vec![args.clone()];

        tokio::spawn(async move {
           let output = tokio::process::Command::new(&cmd).args(args)
                .output().await.unwrap();

        });
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
                        return;
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
