use serde::{Deserialize, Serialize};
use serde_json::Result;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Error;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::{self};
use tokio::time::{sleep, Duration};
use tokio::io::{self};

use self::lsp_messages::{
    CompletionResponse, CompletionResponse2, CompletionResult, 
    DefinitionResponse, DefinitionResult, 
    DiagnosticParams, 
    HoverResponse, HoverResult, 
    ReferencesResponse, ReferencesResult
};

use log2::*;

pub struct Lsp {
    lang: String,
    kill_send: Option<mpsc::Sender<()>>,
    stdin_send: Option<mpsc::Sender<String>>,
    next_id: AtomicUsize,
    versions: HashMap<String, AtomicUsize>,
    pending: Arc<Mutex<HashMap<usize, mpsc::Sender<String>>>>,
    ready: AtomicBool,
    opened: HashSet<String>,
}

impl Lsp {
    pub fn new() -> Self {
        Self {
            lang: String::new(),
            kill_send: None,
            stdin_send: None,
            next_id: AtomicUsize::new(1),
            versions: HashMap::new(),
            pending: Arc::new(Mutex::new(HashMap::new())),
            ready: AtomicBool::new(false),
            opened: HashSet::new(),
        }
    }

    pub fn start(&mut self, lang: &str, cmd: &str, 
        diagnostic_updates: Option<mpsc::Sender<DiagnosticParams>>) 
        ->  io::Result<()>
    {
        // let cmd = match lsp_servers::lang2server(&lang) {
        //     Some(cmd) => cmd,
        //     None => return,
        // };

        let s: Vec<&str> = cmd.split(" ").collect();
        let cmd = s[0];
        let args = &s[1..];

        self.lang = lang.to_string();

        let (kill_send, mut kill_recv) = tokio::sync::mpsc::channel::<()>(1);
        self.kill_send = Some(kill_send);

        let (stdin_send, mut stdin_recv) = tokio::sync::mpsc::channel::<String>(1);
        self.stdin_send = Some(stdin_send);

        // spawn lsp process
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();

        // reading from channel and write to child stdin
        tokio::spawn(async move {
            while let Some(m) = stdin_recv.recv().await {
                debug!("-> {}", m);
                let header = format!("Content-Length: {}\r\n\r\n", m.len());
                stdin.write_all(header.as_bytes()).await;
                stdin.write_all(m.as_bytes()).await;
                stdin.flush().await;
            }
        });

        let pending = self.pending.clone();

        // reading from child stdout
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);

            loop {
                let mut size = None;
                let mut buf = String::new();

                loop {
                    buf.clear();
                    if reader.read_line(&mut buf).await.unwrap_or(0) == 0 { return; }
                    if !buf.ends_with("\r\n") { return; }
                    let buf = &buf[..buf.len() - 2];
                    if buf.is_empty() { break; }
                    let mut parts = buf.splitn(2, ": ");
                    let header_name = parts.next().unwrap();
                    let header_value = parts.next().unwrap();
                    if header_name == "Content-Length" {
                        size = Some(header_value.parse::<usize>().unwrap());
                    }
                }

                let content_length: usize = size.unwrap();
                let mut content = vec![0; content_length];
                reader.read_exact(&mut content).await;

                let msg = std::str::from_utf8(&content).expect("invalid utf8 from server");

                debug!("<- {}", msg);

                let parsed_json: Value = serde_json::from_str(msg).unwrap();

                if let Some(id) = parsed_json["id"].as_u64() { // response
                    let id = id as usize;
                    if let Some(sender) = pending.lock().await.get(&id) {
                        let s = sender.clone();
                        let msg = msg.to_string();
                        tokio::spawn(async move {
                            s.send(msg).await; // send to request channel
                        });
                    } 
                }
                

                if let Some(method) = parsed_json["method"].as_str() {   
                    if method.eq("textDocument/publishDiagnostics") {
                        match serde_json::from_str::<lsp_messages::DiagnosticResponse>(&msg) {
                            Ok(d) => {
                                match diagnostic_updates.as_ref() {
                                    Some(diagnostic_send) => {
                                        diagnostic_send.send(d.params).await;
                                    },
                                    None => {},
                                }
                                
                            },
                            Err(e) => {
                                error!("<- {:?} ", e);
                            },
                        }
                    }   
                }
                  
            }
        });

        // wait for child end or kill
        tokio::spawn(async move {
            tokio::select! {
                _ = child.wait() => {
                    debug!("lsp process wait done");
                }
                _ = kill_recv.recv() => {
                    child.kill().await.expect("kill failed");
                    debug!("lsp process killed manually");
                }
            }
        });

        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(kill_send) = self.kill_send.take() {
            kill_send.send(()).await.expect("Failed to send kill signal");
        }
    }

    fn send_async(&self, message: String) {
        if let Some(stdin_send) = &self.stdin_send {
            let stdin_send = stdin_send.clone();
            tokio::spawn(async move {
                if let Err(err) = stdin_send.send(message).await {
                    error!("Failed to send message: {:?}", err);
                }
            });
        }
    }
    pub async fn add_pending(&mut self, id: usize, sender: mpsc::Sender<String>) {
        self.pending.lock().await.insert(id, sender);
    }
    pub async fn remove_pending(&mut self, id: usize) {
        self.pending.lock().await.remove(&id);
    }

    pub async fn wait_for(&mut self, id: usize, mut rx: mpsc::Receiver<String>) -> Option<String> {
        let timeout = time::sleep(Duration::from_secs(1));
        tokio::pin!(timeout);

        tokio::select! {
            msg = rx.recv() => msg,
            _ = &mut timeout => None
        }
    }
    
    pub async fn init(&mut self, dir: &str) {
        let id = 0;
        let message = lsp_messages::initialize(dir);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;

        self.send_async(message);

        let result = self.wait_for(id, rx).await;
        self.remove_pending(id).await;

        self.initialized();
        tokio::time::sleep(Duration::from_millis(10)).await;
        
        self.ready.store(true, Ordering::SeqCst)
    }

    pub fn is_ready(&mut self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub fn initialized(&mut self) {
        let message = lsp_messages::initialized();
        self.send_async(message);
    }

    pub fn did_open(&mut self, lang: &str, path: &str, text: &str) {
        if self.opened.contains(path) { return; }

        self.opened.insert(path.to_string());

        let message = lsp_messages::did_open(lang, path, text);
        self.send_async(message);
    }

    fn get_next_version(&mut self, path: &str) -> usize { 
        let version = self.versions.entry(path.to_string())
            .or_insert_with(|| AtomicUsize::new(0));
        
        version.fetch_add(1, Ordering::SeqCst)
    }

    fn get_next_id(&mut self, ) -> usize { 
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn did_change(&mut self,
        line: usize,character: usize,
        line_end: usize,character_end: usize,
        path: &str, text: &str,
    ) { 
        if !self.is_ready() { return; }

        let version = self.get_next_version(path);

        let message = lsp_messages::did_change(
            line, character,
            line_end, character_end,
            path, text, version,
        );
        self.send_async(message);
    }

    pub async fn completion(
        &mut self, path: &str, line: usize, character: usize
    ) -> Option<CompletionResult> {
        if !self.is_ready() { return None; }

        let id = self.get_next_id();
        let message = json!({
            "id": id, "jsonrpc": "2.0", "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": format!("file://{}", path) },
                "position": { "line": line, "character": character },
                "context": { "triggerKind": 1 }
            }
        });

        let (tx, rx) = mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;
        self.send_async(message.to_string());

        let result = self.wait_for(id, rx).await;
        self.remove_pending(id).await;

        result.and_then(|message| {
            let res = serde_json::from_str::<CompletionResponse>(&message)
                .map_err(|e| debug!("lsp json parsing error {}", e))
                .ok().and_then(|r| r.result);

            if res.is_some() { res } 
            else {
                // try to parse to CompletionResponse2
                serde_json::from_str::<CompletionResponse2>(&message)
                    .map_err(|e| debug!("lsp json parsing error {}", e))
                    .ok().and_then(|r| Some(CompletionResult { 
                        isIncomplete: Some(false), items: r.result 
                    }))
            }
        })
    }

    pub async fn definition(
        &mut self, path: &str, line: usize, character: usize
    ) -> Option<Vec<DefinitionResult>> {
        if !self.is_ready() { return None; }

        let id = self.get_next_id();
        let message = json!({
            "id": id, "jsonrpc": "2.0", "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": format!("file://{}", path) },
                "position": { "line": line, "character": character },
            }
        });

        let (tx, rx) = mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;
        self.send_async(message.to_string());

        let result = self.wait_for(id, rx).await;
        self.remove_pending(id).await;

        result.and_then(|message| {
            serde_json::from_str::<DefinitionResponse>(&message)
                .map_err(|e| debug!("lsp json parsing error {}", e))
                .ok().and_then(|r| Some(r.result))
        })
    }
    
    pub async fn references(
        &mut self, path: &str, line: usize, character: usize
    ) -> Option<Vec<ReferencesResult>> {
        if !self.is_ready() { return None; }

        let id = self.get_next_id();
        let message = json!({
            "id": id, "jsonrpc": "2.0", "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": format!("file://{}", path) },
                "position": { "line": line, "character": character },
                "context" : { "includeDeclaration": false }
            }
        });

        let (tx, rx) = mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;
        self.send_async(message.to_string());

        let result = self.wait_for(id, rx).await;
        self.remove_pending(id).await;

        result.and_then(|message| {
            serde_json::from_str::<ReferencesResponse>(&message)
                .map_err(|e| debug!("lsp json parsing error {}", e))
                .ok().and_then(|r| Some(r.result))
        })
    }


    pub async fn hover(
        &mut self, path: &str, line: usize, character: usize
    ) -> Option<HoverResult> {
        if !self.is_ready() { return None; }

        let id = self.get_next_id();
        let message = json!({
            "id": id, "jsonrpc": "2.0", "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": format!("file://{}", path) },
                "position": { "line": line, "character": character },
            }
        });

        let (tx, rx) = mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;
        self.send_async(message.to_string());

        let result = self.wait_for(id, rx).await;
        self.remove_pending(id).await;

        result.and_then(|message| {
            serde_json::from_str::<HoverResponse>(&message)
                .map_err(|e| debug!("lsp json parsing error {}", e))
                .ok().and_then(|r| Some(r.result))
        })
    }
}

#[tokio::test]
async fn test_lsp() {
    let lang = "rust";
    let mut lsp = Lsp::new();

    lsp.start(lang, "rust-analyzer", None);
    println!("after lsp start");

    sleep(Duration::from_secs(2)).await;

    let dir = "/Users/max/apps/rust/red";
    lsp.init(dir);
    println!("after lsp init");

    sleep(Duration::from_secs(2)).await;

    lsp.initialized();
    println!("after lsp initialized");

    sleep(Duration::from_secs(3)).await;

    let file_name = format!("{}/src/main.rs", dir);
    let file_content = std::fs::read_to_string(&file_name).unwrap();

    lsp.did_open(lang, &file_name, &file_content);
    println!("after lsp did_open");

    sleep(Duration::from_secs(5)).await;

    let cr = lsp.completion(&file_name, 17 - 1, 17 - 1).await;
    println!("after lsp completion");

    match cr {
        Some(result) => {
            for item in result.items {
                println!("{}", item.label)
            }
        }
        None => {}
    }

    sleep(Duration::from_secs(3)).await;

    lsp.stop().await;
    println!("after stop");

    sleep(Duration::from_secs(3)).await;
}

#[tokio::test]
async fn main() {
    let (send, recv) = tokio::sync::oneshot::channel::<()>();
    let (send2, mut recv2) = tokio::sync::mpsc::channel::<String>(1);
    let send2_arc = Arc::new(send2).clone();

    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("command not found");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // sending to child stdin task
    tokio::spawn(async move {
        while let Some(m) = recv2.recv().await {
            println!("sending message to child stdin {}", m);
            stdin.write_all(m.as_bytes()).await;
        }
    });

    // reading child stdout task
    tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        println!("reading stdout task start");
        while let Ok(Some(line)) = lines.next_line().await {
            println!("Received line from child: {}", line);
        }
        println!("reading stdout task end");
    });

    tokio::spawn(async move {
        sleep(Duration::from_secs(3)).await;
        send2_arc.send("hello\n".to_string()).await;
    });

    // tokio::spawn(async move {
    //     sleep(Duration::from_secs(2)).await;
    //     println!("sending kill");
    //     send.send(())
    // });

    // wait for process stopped or killed
    tokio::select! {
        _ = child.wait() => {
            println!("child wait done, exited");
        }
        _ = recv => {
            child.kill().await.expect("kill failed");
            println!("killed");
        }
    }
}

pub mod lsp_messages {
    use serde::{Deserialize, Serialize};
    use serde_json::Result;
    use serde_json::{json, Value};

    // todo, replace it to struct in the future

    pub fn initialize(dir: &str) -> String {
        json!({
            "id": 0,
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {
                "rootPath": dir,
                "rootUri": format!("file://{}", dir),
                "processId": std::process::id(),
                "workspaceFolders": [
                    {
                        "name": std::path::Path::new(dir).file_name().unwrap().to_str(),
                        "uri": format!("file://{}", dir)
                    }
                ],
                "clientInfo": {
                    "name": "red",
                    "version": "1.0.0"
                },
                "capabilities": {
                    "textDocument": {
                        "synchronization": {
                            "dynamicRegistration": true,
                        },
                        "hover": {
                            "contentFormat": [
                                "plaintext",
                            ]
                        },
                        "publishDiagnostics": {
                            "relatedInformation": false,
                            "versionSupport": false,
                            "codeDescriptionSupport": true,
                            "dataSupport": true
                        },
                        "signatureHelp": {
                            "signatureInformation": {
                                "documentationFormat": [
                                    "plaintext",
                                ]
                            }
                        },
                        "completion": {
                            "completionItem": {
                                "resolveProvider": true,
                                "snippetSupport": false,
                                "insertReplaceSupport": true,
                                "labelDetailsSupport": true,
                                "resolveSupport": {
                                    "properties": [
                                        "documentation",
                                        "detail",
                                        "additionalTextEdits"
                                    ]
                                }
                            }
                        }
                    }
                }
            }
        })
        .to_string()
    }

    pub fn initialized() -> String {
        json!({"jsonrpc": "2.0","method": "initialized","params": {}}).to_string()
    }

    pub fn did_change_configuration() -> String {
        json!({
            "jsonrpc":"2.0",
            "method":"workspace/didChangeConfiguration",
            "params":{
                "settings":{"hints":{
                    "assignVariableTypes":true,
                    "compositeLiteralFields":true,
                    "constantValues":true,
                    "functionTypeParameters":true,
                    "parameterNames":true,
                    "rangeVariableTypes":true
                    }
                }
            }
        })
        .to_string()
    }

    pub fn did_open(lang: &str, path: &str, text: &str) -> String {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "languageId": lang,
                    "text": text,
                    "uri": format!("file://{}", path),
                    "version": 0
                }
            }
        })
        .to_string()
    }

    pub fn did_change_watched_files(path: &str) -> String {
        json!({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeWatchedFiles",
            "params": {
                "changes":[
                    { "uri":format!("file://{}", path), "type":2 }
                    ]

            }
        })
        .to_string()
    }

    pub fn document_link(path: &str) -> String {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/documentLink",
            "params": {
                "textDocument": {
                    "uri": format!("file://{}", path),
                }
            }
        })
        .to_string()
    }

    pub fn did_change(
        line: usize,
        character: usize,
        line_end: usize,
        character_end: usize,
        path: &str,
        text: &str,
        version: usize,
    ) -> String {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "contentChanges": [
                {
                    "range": {
                        "start": {
                            "line": line,
                            "character": character
                        },
                        "end": {
                            "line": line_end,
                            "character": character_end
                        }
                    },
                    "text": text
                }
                ],
                "textDocument": {
                    "uri": format!("file://{}", path),
                    "version": version
                }
            }
        })
        .to_string()
    }

    pub fn completion(id: usize, path: &str, line: usize, character: usize) -> String {
        json!({
            "id": id, "jsonrpc": "2.0", "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": format!("file://{}", path) },
                "position": { "line": line, "character": character },
                "context": { "triggerKind": 1 }
            }
        })
        .to_string()
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct CompletionResponse {
        pub jsonrpc: String,
        #[serde(default)]
        pub result: Option<CompletionResult>,
        pub id: f64,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct CompletionResponse2 {
        pub jsonrpc: String,
        #[serde(default)]
        pub result: Vec<CompletionItem>,
        pub id: f64,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct CompletionResult {
        pub isIncomplete: Option<bool>,
        pub items: Vec<CompletionItem>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct CompletionItem {
        pub label: String,
        pub kind: f64,
        pub detail: Option<String>,
        pub preselect: Option<bool>,
        pub sortText: Option<String>,
        pub insertText: Option<String>,
        pub filterText: Option<String>,
        pub insertTextFormat: Option<f64>,
        pub textEdit: Option<TextEdit>,
        pub data: Option<serde_json::Value>, 
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct TextEdit {
        pub range: Option<Range>,
        pub replace: Option<Range>,
        pub insert: Option<Range>,
        pub newText: String,
    }
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Range {
        pub start: PositionResponse,
        pub end: PositionResponse,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct PositionResponse {
        pub line: f64,
        pub character: f64,
    }


    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct DiagnosticResponse {
        pub jsonrpc: String,
        pub method: String,
        pub params: DiagnosticParams,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct DiagnosticParams {
        pub uri: String,
        pub version: Option<i32>,
        pub diagnostics: Vec<Diagnostic>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Diagnostic {
        pub range: Range,
        pub severity: i32,
        pub code: Option<serde_json::Value>, 
        pub code_description: Option<CodeDescription>,
        pub source: String,
        pub message: String,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct CodeDescription {
        pub href: String,
    }


    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct DefinitionResponse {
        pub jsonrpc: String,
        pub result: Vec<DefinitionResult>,
        pub id: f64,
    }
    
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct DefinitionResult {
        pub uri: String,
        pub range: Range,
    }


    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct ReferencesResponse {
        pub jsonrpc: String,
        pub result: Vec<ReferencesResult>,
        pub id: f64,
    }
        
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct ReferencesResult {
        pub uri: String,
        pub range: Range,
    }


    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct HoverResponse {
        pub jsonrpc: String,
        pub result: HoverResult,
        pub id: f64,
    }
        
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct HoverResult {
        pub contents: Contents,
        pub range: Range,
    }
    
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Contents {
        pub kind: String,
        pub value: String,
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;
    use tests::lsp_messages::CompletionResponse2;

    #[test]
    fn test_deserialization() {
        // JSON input string
        let json_str = r#"{
            "jsonrpc": "2.0",
            "id": 1,
            "result": [
                {
                    "label": "echo",
                    "kind": 3,
                    "data": {
                        "type": 0
                    }
                },
                {
                    "documentation": {
                        "value": "```man\n\"echo\" invocation (bash-language-server)\n\n\n```\n```bash\necho \"${1:message}\"\n```",
                        "kind": "markdown"
                    },
                    "label": "echo",
                    "insertText": "echo \"${1:message}\"",
                    "insertTextFormat": 2,
                    "data": {
                        "type": 4
                    },
                    "kind": 15
                }
            ]
        }"#;

        // Deserialize JSON into Rust structs
        let completion_response: CompletionResponse2 = serde_json::from_str(json_str).unwrap_or_else(|e| {
            panic!("Failed to deserialize JSON: {}", e);
        });

        // Print the deserialized structs
        println!("{:#?}", completion_response);
    }
}
