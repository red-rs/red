mod code;
mod editor;
mod lsp;
mod process;
mod search;
mod selection;
mod tree;
mod tests;
mod utils;
mod config;
mod screen;

use editor::Editor;

use log2::*;

#[tokio::main]
async fn main() {
    let current_dir = utils::current_directory_name().unwrap();

    let logger = match std::env::var("RED_LOG") {
        Ok(p) => Some(log2::open(&p).start()),
        Err(_) => None,
    };

    debug!("starting red");

    let mut editor = Editor::new(current_dir, config::get());

    editor.handle_panic();

    match std::env::args().nth(1) {
        None => editor.open_left_panel(),
        Some(path) if path == "." || path == "./" =>
            editor.open_left_panel(),
        Some(path) => {
            editor.close_left_panel();
            editor.load_file(&path);
        }
    }

    editor.start().await;

    debug!("stopping red");
}
