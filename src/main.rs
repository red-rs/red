mod code;
mod editor;
mod lsp;
mod process;
mod search;
mod selection;
mod tree;
mod utils;
mod config;
use editor::Editor;

use log2::*;

#[tokio::main]
async fn main() {
    if let Some(arg) = std::env::args().nth(1) {
        if arg == "--help" || arg == "-h" {
            print_help();
            std::process::exit(0);
        }
    }

    let _logger = match std::env::var("RED_LOG") {
        Ok(p) => Some(log2::open(&p).start()),
        Err(_) => None,
    };

    debug!("starting red");

    let mut editor = Editor::new(config::get());

    editor.handle_panic();

    match std::env::args().nth(1) {
        None => editor.open_left_panel(),
        Some(path) if path == "." || path == "./" =>
            editor.open_left_panel(),
        Some(path) => {
            editor.close_left_panel();
            editor.load_file(&path);
            editor.save_cursor_to_history();
        }
    }

    editor.start().await;

    debug!("stopping red");
}

fn print_help() {
    let help = r#"red is a console-based text editor designed to be simple and efficient.

USAGE: red [file]
If [file] is provided, opens the file. Otherwise, opens current folder.

OPTIONS:
  -h, --help     Show this help message and exit

KEY BINDINGS:
  Ctrl+q                  Quit
  Ctrl+s                  Save
  Ctrl+c                  Copy
  Ctrl+v                  Paste
  Ctrl+x                  Cut
  Ctrl+d                  Duplicate
  Ctrl+z                  Undo
  Ctrl+y                  Redo
  Ctrl+f                  Find
  Ctrl+f, type, Ctrl+g    Global find
  Ctrl+o                  Cursor back
  Ctrl+p                  Cursor forward
  Shift+arrow             Select text
  Option+right/left       Smart horizontal movement
  Option+down/up          Smart selection
  Option+delete           Delete line
  Option+/                Comment line
  Ctrl+Shift+down/up      Lines swap
  Mouse selection         Select text
  Mouse double click      Select word
  Mouse triple click      Select line
  Ctrl+space              LSP completion
  Ctrl+h                  LSP hover
  Ctrl+g / Ctrl+mouse     LSP definition
  Ctrl+r / Option+mouse   LSP references
  Ctrl+e                  LSP diagnostic (errors)

For more, see readme.md or source code at https://github.com/red-rs/red.
"#;
    println!("{}", help);
}