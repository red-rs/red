use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use anyhow::Result;

pub struct Process {
    last_cmd: String
}

impl Process {
    pub fn new() -> Self {
        Self {
            last_cmd: String::new(),
        }
    }
    
    /// Run command in tmux pane
    /// It creates a new tmux pane and runs the command in it
    /// If pane exists, it runs the command in the existing pane
    pub async fn run_tmux(&mut self, args: &String) -> Result<()> {
        let args_vec = vec![args.clone()];
        self.last_cmd = args_vec.join(" ");

        let user_cmd = args.clone();

        let script = r#"
            PANES=$(tmux list-panes | wc -l)
            
            if [ "$PANES" -le 1 ]; then
            tmux split-window -v
            fi
            
            tmux send-keys -t 1 "$@" Enter
        "#;

        let mut child = Command::new("sh")
            .arg("-s") // read script from stdin
            .arg(&user_cmd) 
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(script.as_bytes()).await;
        }

        let status = child.wait().await;
        if let Err(e) = status {
            eprintln!("Script failed: {}", e);
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn run_last_tmux(&mut self) -> Result<()> {
        if self.last_cmd.is_empty() { return Ok(()) }
        let last_cmd = self.last_cmd.clone();
        self.run_tmux(&last_cmd).await
    }
}
