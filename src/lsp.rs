use serde::{Deserialize, Serialize};
use serde_json::{Value};
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{self};
use tokio::time::{Duration};
use tokio::io::{self};
use log2::{info, debug, error};

use lsp_types::*;
use lsp_types::notification::*;

use crate::config::Config;

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

    pub fn start(
        &mut self, lang: &str, cmd: &str,
        diagnostic_updates: Option<mpsc::Sender<PublishDiagnosticsParams>>
    ) -> io::Result<()> {

        let s: Vec<&str> = cmd.split(" ").collect();
        let cmd = s[0];
        let args = &s[1..];

        self.lang = lang.to_string();

        let (kill_send, mut kill_recv) = mpsc::channel::<()>(1);
        self.kill_send = Some(kill_send);

        let (stdin_send, mut stdin_recv) = mpsc::channel::<String>(1);
        self.stdin_send = Some(stdin_send);

        // spawn lsp process
        let mut child = Command::new(cmd)
            
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // reading from channel and write to child stdin
        tokio::spawn(async move {
            while let Some(m) = stdin_recv.recv().await {
                info!("-> {}", m);
                let header = format!("Content-Length: {}\r\n\r\n", m.len());
                let _ = stdin.write_all(header.as_bytes()).await;
                let _ = stdin.write_all(m.as_bytes()).await;
                let _ = stdin.flush().await;
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
                let _ = reader.read_exact(&mut content).await;

                let msg = std::str::from_utf8(&content)
                    .expect("invalid utf8 from lsp server");

                info!("<- {}", msg);

                let parsed_json: Value = serde_json::from_str(msg).unwrap();

                if let Some(id) = parsed_json["id"].as_u64() { // response
                    let id = id as usize;
                    if let Some(sender) = pending.lock().await.get(&id) {
                        let _ = sender.send(msg.to_string()).await;
                    }
                    continue;
                }

                match parsed_json.get("method").and_then(|v| v.as_str()) {
                    Some("textDocument/publishDiagnostics") => { // diagnostics
                        let v = parsed_json["params"].clone();
                        if let Ok(params) = serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(v) {
                            if let Some(sender) = diagnostic_updates.as_ref() {
                                let _ = sender.send(params).await;
                                continue;
                            }
                        }
                    }
                    _ => {}
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

    #[allow(dead_code)]
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

    pub async fn wait(
        &mut self, timeout: usize, mut rx: mpsc::Receiver<String>
    ) -> Option<String> {
        let timeout = time::sleep(Duration::from_secs(timeout as u64));
        tokio::pin!(timeout);

        tokio::select! {
            msg = rx.recv() => msg,
            _ = &mut timeout => None
        }
    }

    pub async fn init(&mut self, dir: &str) {
        let id = 0;
        let (tx, rx) = mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;
        let message = lsp_messages::initialize(dir);
        self.send_async(message);
        self.wait(5, rx).await;
        self.remove_pending(id).await;
        self.initialized();
        self.ready.store(true, Ordering::SeqCst)
    }

    pub fn send_notification<N>(&self, params: N::Params)
    where
        N: lsp_types::notification::Notification,
        N::Params: Serialize,
    {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": N::METHOD,
            "params": params
        });

        self.send_async(msg.to_string());
    }

    pub async fn send_request<R>(
        &mut self, params: R::Params
    ) -> anyhow::Result<R::Result>
    where
        R: lsp_types::request::Request,
        R::Params: Serialize,
        R::Result: for<'de> serde::Deserialize<'de>,
    {
        if !self.is_ready() {
            return Err(anyhow::anyhow!("LSP not ready"));
        }

        let id = self.get_next_id();

        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "method": R::METHOD,
            "params": serde_json::to_value(params)?,
        });

        let (tx, rx) = mpsc::channel::<String>(1);
        self.add_pending(id, tx).await;
        self.send_async(msg.to_string());
        let response = self.wait(3, rx).await;
        self.remove_pending(id).await;

        let response_str = response.ok_or_else(||
            anyhow::anyhow!("no response for request {}", R::METHOD))?;

        let raw: lsp_messages::LspRawResponse = serde_json::from_str(&response_str)?;

        if let Some(err) = raw.error {
            return Err(anyhow::anyhow!("LSP error: {}", err));
        }

        let result_value = raw.result.ok_or_else(||
            anyhow::anyhow!("missing result field"))?;

        let parsed = serde_json::from_value::<R::Result>(result_value)?;

        Ok(parsed)
    }

    pub fn is_ready(&mut self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub fn initialized(&mut self) {
        let params = InitializedParams {};
        self.send_notification::<Initialized>(params);
    }

    pub fn did_open(&mut self, lang: &str, path: &str, text: &str) {
        self.opened.insert(path.to_string());

        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: format!("file://{}", path).parse().unwrap(),
                language_id: lang.to_string(),
                version: 0,
                text: text.to_string(),
            },
        };
        self.send_notification::<DidOpenTextDocument>(params);
    }

    #[allow(dead_code)]
    pub fn did_close(&mut self, path: &str) {
        if !self.opened.remove(path) {
            return;
        }
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: format!("file://{}", path).parse().unwrap()
            },
        };
        self.send_notification::<DidCloseTextDocument>(params);
    }

    fn get_next_version(&mut self, path: &str) -> usize {
        let version = self.versions.entry(path.to_string())
            .or_insert_with(|| AtomicUsize::new(0));

        version.fetch_add(1, Ordering::SeqCst)
    }

    fn get_next_id(&mut self, ) -> usize {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn did_change(
        &mut self,
        start_line: usize, start_column: usize,
        end_line: usize, end_column: usize,
        path: &str, text: &str,
    ) {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: format!("file://{}", path).parse().unwrap(),
                version: self.get_next_version(path) as i32,
            },
            content_changes: vec![
                TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position::new(start_line as u32, start_column as u32),
                        end: Position::new(end_line as u32, end_column as u32),
                    }),
                    range_length: None,
                    text: text.to_string(),
                }
            ],
        };

        self.send_notification::<DidChangeTextDocument>(params);
    }

    pub async fn completion(
        &mut self, path: &str, line: usize, character: usize
    ) -> anyhow::Result<Vec<CompletionItem>> {

        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: format!("file://{}", path).parse().unwrap(),
                },
                position: Position::new(line as u32, character as u32),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: Some(CompletionContext {
                trigger_kind: CompletionTriggerKind::INVOKED,
                trigger_character: None,
            }),
        };

        let response = self
            .send_request::<lsp_types::request::Completion>(params)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Completion returned None"))?;

        let items = match response {
            lsp_types::CompletionResponse::Array(items) => items,
            lsp_types::CompletionResponse::List(list) => list.items,
        };

        Ok(items)
    }

    pub async fn definition(
        &mut self, path: &str, line: usize, character: usize,
    ) -> anyhow::Result<Vec<Location>> {
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: format!("file://{}", path).parse()?,
                },
                position: Position::new(line as u32, character as u32),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let response = self
            .send_request::<lsp_types::request::GotoDefinition>(params)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Definition returned None"))?;

        let locations = match response {
            lsp_types::GotoDefinitionResponse::Scalar(location) => vec![location],
            lsp_types::GotoDefinitionResponse::Array(locations) => locations,
            lsp_types::GotoDefinitionResponse::Link(links) => {
                links.into_iter()
                    .map(|l| Location::new(l.target_uri, l.target_range))
                    .collect()
            }
        };

        Ok(locations)
    }

    pub async fn references(
        &mut self, path: &str, line: usize, character: usize,
    ) -> anyhow::Result<Vec<Location>> {
        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: format!("file://{}", path).parse()?,
                },
                position: Position::new(line as u32, character as u32),
            },
            context: ReferenceContext {
                include_declaration: false,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let response = self
            .send_request::<lsp_types::request::References>(params)
            .await?
            .ok_or_else(|| anyhow::anyhow!("References returned None"))?;

        Ok(response)
    }

    pub async fn hover(
        &mut self, path: &str, line: usize, character: usize,
    ) -> anyhow::Result<Hover> {

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: format!("file://{}", path).parse()?,
                },
                position: Position::new(line as u32, character as u32),
            },
            work_done_progress_params: Default::default(),
        };

        let response = self
            .send_request::<lsp_types::request::HoverRequest>(params)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Hover returned None"))?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_lsp_minimal() -> anyhow::Result<()> {
        let lang = "python";

        let mut lsp = Lsp::new();
        lsp.start(lang, "pyright-langserver --stdio", None)?;

        let dir = std::env::current_dir().unwrap()
            .to_string_lossy().into_owned();

        lsp.init(&dir).await;

        let content = r#"for i in range(10000): print(i)"#;
        let file_path = "fast.py";

        lsp.did_open(lang, file_path, content);

        // Test completion on 'range'
        let completions = lsp.completion(file_path, 0, 12).await?;
        let completions_str = format!("{:?}", completions);
        // println!("Completions: {:?}", completions_str);
        assert!(!completions.is_empty());
        assert!(completions_str.contains("label: \"range\""));

        // Test hover on 'range'
        let hover = lsp.hover(file_path, 0, 12).await?;
        let hover_str = format!("{:?}", hover.contents);
        // println!("Hover: {:?}", hover_str);
        assert!(hover_str.contains("class range"));
        
        // Test definition on 'i'  
        let definitions = lsp.definition(file_path, 0, 30).await?; 
        let definition_str = format!("{:?}", definitions);
        // println!("Definitions: {:?}", definition_str);
        assert!(definition_str.contains("fast.py"));
        assert!(definition_str.contains("Position { line: 0, character: 5 }"));
        
        // Test references on 'i'
        let references = lsp.references(file_path, 0, 4).await?;
        let references_str = format!("{:?}", references);
        // println!("References: {:?}", references_str);
        assert!(references_str.contains("fast.py"));
        assert!(references_str.contains("Position { line: 0, character: 30 }"));

        lsp.stop().await;
        Ok(())
    }

}

pub mod lsp_messages {
    use super::*;
    use serde_json::to_string;

    #[allow(dead_code)]
    #[derive(Deserialize)]
    pub struct LspRawResponse {
        pub jsonrpc: String,
        pub id: Value,
        pub result: Option<Value>,
        pub error: Option<Value>,
    }

    pub fn initialize(dir: &str) -> String {
        let uri: Uri = format!("file://{}", dir).parse().unwrap();

        let workspace_folders = Some(vec![
            WorkspaceFolder {
                name: std::path::Path::new(dir)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
                uri: uri.clone(),
            }
        ]);

        let capabilities = ClientCapabilities {
            text_document: Some(TextDocumentClientCapabilities {
                hover: Some(lsp_types::HoverClientCapabilities {
                    content_format: Some(vec![lsp_types::MarkupKind::PlainText]),
                    ..Default::default()
                }),
                synchronization: Some(Default::default()),
                signature_help: Some(lsp_types::SignatureHelpClientCapabilities {
                    signature_information: Some(lsp_types::SignatureInformationSettings {
                        documentation_format: Some(vec![lsp_types::MarkupKind::PlainText]),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                completion: Some(lsp_types::CompletionClientCapabilities {
                    completion_item: Some(lsp_types::CompletionItemCapability {
                        insert_replace_support: Some(true),
                        label_details_support: Some(true),
                        snippet_support: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                publish_diagnostics: Some(lsp_types::PublishDiagnosticsClientCapabilities {
                    related_information: Some(false),
                    version_support: Some(false),
                    code_description_support: Some(true),
                    data_support: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let params = InitializeParams {
            process_id: Some(std::process::id() as u32),
            // root_path: Some(dir.to_string()),
            // root_uri: Some(uri),
            capabilities,
            workspace_folders,
            client_info: Some(ClientInfo {
                name: "anycode".to_string(),
                version: Some("1.0.0".to_string()),
            }),
            ..Default::default()
        };

        let request = serde_json::json!({
            "id": 0,
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": params
        });

        to_string(&request).unwrap()
    }
}

#[allow(dead_code)]
pub struct LspManager {
    config: Config,
    lang2lsp: HashMap<String,Lsp>,
    diagnostics_sender: Option<mpsc::Sender<PublishDiagnosticsParams>>,
}

impl LspManager {
    #[allow(dead_code)]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            lang2lsp: HashMap::new(),
            diagnostics_sender: None,
        }
    }

    #[allow(dead_code)]
    pub fn set_diagnostics_sender(&mut self, d: mpsc::Sender<PublishDiagnosticsParams>) {
        self.diagnostics_sender = Some(d);
    }

    #[allow(dead_code)]
    pub async fn get(&mut self, lang: &str) -> Option<&mut Lsp> {

        let lang_conf = self.config.language.iter().find(|lang_conf| lang_conf.name == lang)?;
        let cmd = lang_conf.clone().lsp?.join(" ");

        if !self.lang2lsp.contains_key(lang) {
           self.init_new(lang.to_string(), &cmd).await;
        }

        self.lang2lsp.get_mut(lang)
    }

    #[allow(dead_code)]
    pub async fn init_new(&mut self, lang: String, lsp_cmd: &str) {
        let mut lsp = Lsp::new();
        let diagnostic_send = self.diagnostics_sender.as_mut().map(|s|s.clone());
        let result = lsp.start(&lang, &lsp_cmd, diagnostic_send);

        match result {
            Ok(_) => {
                info!("lsp process started {}", &lsp_cmd);
            },
            Err(e) => {
                error!("error starting lsp process {}: {}", &lsp_cmd, e.to_string());
                // panic!("error starting lsp process {}", e.to_string());
                return;
            },
        }

        let dir = std::env::current_dir().unwrap()
            .to_string_lossy().into_owned();

        lsp.init(&dir).await;

        self.lang2lsp.insert(lang, lsp);
    }
}