use std::cmp::Ordering;
// editor.rs
use std::collections::{HashMap, HashSet};
use std::io::{stdout, Write};
use std::path::Path;
use std::time::Instant;
use std::{fs, time};
use log2::debug;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind, KeyEventKind
};
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{
    cursor,
    cursor::position,
    event, execute, queue,
    style::{Color, SetBackgroundColor as BColor, SetForegroundColor as FColor},
    terminal,
};

use futures::{future::FutureExt, select, StreamExt};

use crate::code::{Code, NodePath, Runnable};
use crate::config::Config;
use crate::search::search::FileSearchResult;
use crate::search::{Search, SearchResult};
use crate::lsp::{self, Lsp};
use crate::lsp::lsp_messages::{CompletionItem, Diagnostic, DiagnosticParams, HoverResult, ReferencesResult};

use crate::process::Process;
use crate::selection::Selection;
use crate::utils::{CursorHistory, CursorPosition};
use crate::{search, utils};
use crate::tree;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;

use std::sync::Arc;

use copypasta::{ClipboardContext, ClipboardProvider};

/// Represents a text editor.
pub struct Editor {
    /// Config from config.toml.
    config: Config,
    /// Text buffer to display.
    code: Code,
    /// Terminal height.
    height: usize,
    /// Terminal width.
    width: usize,

    /// Cursor row.
    r: usize,
    /// Cursor column.
    c: usize,
    /// Cursor row offset.
    x: usize,
    /// Cursor column offset.
    y: usize,

    /// Left panel width
    lp_width: usize,
    /// Line number width.
    ln_width: usize,
    /// Line number to text space.
    lns_width: usize,

    /// Update screen flag.
    upd: bool,
    upd_next: bool,

    /// Theme for syntax highlighting and etc
    theme: HashMap<String, String>,

    /// Color for line number.
    lncolor: Color,
    /// Color for status line.
    scolor: Color,
    /// Color for selection.
    selcolor: Color,
    /// Color for errors.
    ecolor: Color,

    /// Mouse selection range.
    selection: Selection,

    /// process
    process: Process,

    /// lsp servers for a language
    lang2lsp: HashMap<String,Arc<Mutex<Lsp>>>,
    lsp_status: Arc<Mutex<String>>,

    /// diagnostics or errors to inline display
    diagnostics: Arc<Mutex<HashMap<String, DiagnosticParams>>>,
    diagnostics_sender: Option<tokio::sync::mpsc::Sender<DiagnosticParams>>,

    /// tree view
    tree_view: tree::TreeView,

    /// opened text buffers
    codes: HashMap<String, Code>,

    /// search
    search: Search,

    overlay_lines: HashSet<usize>,

    /// cursor position between files switches and mouse clicks
    cursor_history: CursorHistory,
    cursor_history_undo: CursorHistory,

    is_lp_focused: bool,

    node_path: Option<NodePath>,
}

impl Editor {
    pub fn new(dir: String, config: Config) -> Self {
        Editor {
            config,
            code: Code::new(),
            height: 0,
            width: 0,
            ln_width: 5,
            lns_width: 5,
            r: 0, c: 0, x: 0, y: 0,
            lncolor: Color::Reset,
            scolor: Color::Reset,
            selcolor: Color::Reset,
            ecolor: Color::Reset,
            upd: true,
            upd_next: false,
            theme: HashMap::new(),
            selection: Selection::new(),
            process: Process::new(),
            lang2lsp: HashMap::new(),
            lsp_status: Arc::new(Mutex::new(String::new())),
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
            diagnostics_sender: None,
            tree_view: tree::TreeView::new(".".to_string()),
            lp_width: 0,
            codes: HashMap::new(),
            search:Search::new(),
            overlay_lines: HashSet::new(),
            cursor_history: CursorHistory::new(),
            cursor_history_undo: CursorHistory::new(),
            is_lp_focused: false,
            node_path: None,
        }
    }

    pub fn load_file(&mut self, fname: &str) {
        let buf = Code::from_file(&fname, &self.config);

        match buf {
            Ok(code) => {
                self.code = code;
                self.r = 0; self.c = 0; self.y = 0; self.x = 0;
                self.selection.clean();
            }
            Err(e) => {},
        }
    }

    pub fn open_left_panel(&mut self) {
        self.lp_width = self.config.left_panel_width.unwrap_or(25);
        self.is_lp_focused = true;
    }
    pub fn close_left_panel(&mut self) {
        self.lp_width = 0;
    }
    pub fn left_panel_toggle(&mut self) {
        if self.lp_width > 0 { self.lp_width = 0; }
        else { self.lp_width = self.config.left_panel_width.unwrap_or(25); }

        self.tree_view.set_width(self.lp_width);
    }

    pub fn init(&mut self) {
        execute!(stdout(), EnterAlternateScreen).expect("Could not EnterAlternateScreen");
        execute!(stdout(), EnableMouseCapture).expect("Could not EnableMouseCapture");
        enable_raw_mode().expect("Could not turn on Raw mode");
        execute!(stdout(), cursor::Hide).expect("Could not hide cursor");
        stdout().flush().expect("Could not flush");
        let (w, h) = terminal::size().expect("Could not get screen size");
        self.resize(w as usize, h as usize);
        self.tree_view.set_width(self.lp_width);

        self.configure_theme();
    }

    pub fn deinit() {
        disable_raw_mode().expect("Unable to disable_raw_mode");
        execute!(stdout(), LeaveAlternateScreen).expect("Unable to LeaveAlternateScreen");
        execute!(stdout(), DisableMouseCapture).expect("Unable DisableMouseCapture");
        queue!(stdout(), cursor::Show).expect("Unable to show cursor");
    }

    pub fn handle_panic(&self) {
        let default_panic = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Self::deinit();
            default_panic(info);
            std::process::exit(1);
        }));

        ctrlc::set_handler(move || {
            Self::deinit();
        })
        .expect("Error setting Ctrl-C handler");
    }

    fn configure_theme(&mut self) {
        let theme_path = &self.config.theme;
        let path = Path::new(theme_path);

        let theme_path = if path.is_absolute() {
           path.to_string_lossy().to_string()
        } else {
            let red_home = option_env!("RED_HOME").unwrap_or("./");
            Path::new(red_home).join(theme_path).to_string_lossy().to_string()
        };

        let theme_content = fs::read_to_string(theme_path).expect("Failed to read theme path file");
        let theme_yaml = serde_yaml::from_str(&theme_content).expect("Failed to parse theme yaml file ");
        self.theme = utils::yaml_to_map(theme_yaml);

        self.lncolor = self.theme.get("lncolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::AnsiValue(247));
        self.scolor = self.theme.get("scolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::AnsiValue(247));
        self.selcolor = self.theme.get("selcolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::AnsiValue(247));
        self.ecolor = self.theme.get("ecolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::AnsiValue(247));

        let dircolor = self.theme.get("dircolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::Reset);
        self.tree_view.set_dir_color(dircolor);
        let filecolor = self.theme.get("filecolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::Reset);
        self.tree_view.set_file_color(filecolor);
        let activefilecolor = self.theme.get("activefilecolor").map(|c| utils::hex_to_color(c)).unwrap_or(Color::Reset);
        self.tree_view.set_active_file_color(activefilecolor);
    }

    pub async fn start(&mut self) {
        self.init();

        self.draw().await;

        let (diagnostic_send, mut diagnostic_recv) = tokio::sync::mpsc::channel::<DiagnosticParams>(1);
        self.diagnostics_sender = Some(diagnostic_send.clone());

        self.init_new_lsp();

        let mut reader = EventStream::new();

        loop {
            // let delay = Delay::new(Duration::from_millis(1_00)).fuse();
            let event = reader.next().fuse();

            tokio::select! {
                Some(upd) = diagnostic_recv.recv() => {
                    let filename = upd.uri.clone();
                    self.diagnostics.lock().await.insert(filename, upd);
                    self.upd = true;
                    self.draw().await;
                }
                // _ = delay => {
                    // println!(".\r");
                    // let upd_process = self.upd_process.clone();
                    // let mut upd_process = upd_process.lock().expect("cant get lock");

                    // if self.process.upd() {
                    //     self.draw_process();
                    //     self.draw_status();
                    //     self.draw_cursor();
                    //     // *upd_process = false;
                    // }
                // },

                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            // println!("Event::{:?}\r", event);

                            match event {
                                Event::Resize(w, h) => {
                                    self.resize(w as usize, h as usize);
                                    self.draw().await;
                                }
                                Event::Mouse(e) => {
                                    self.handle_mouse(e).await;
                                    self.draw().await;
                                }
                                Event::Key(e) => {
                                    if self.is_quit(e) { break }

                                    #[cfg(target_os = "windows")] { // skip press event on windows
                                        if e.kind == KeyEventKind::Press { continue; }
                                    };
                                    self.handle_keyboard(e).await;

                                    self.draw().await;
                                    if self.upd_next {
                                        self.upd = true;
                                        self.upd_next = false;
                                    }
                                }
                                Event::FocusGained => {}
                                Event::FocusLost => {}
                                Event::Paste(_) => {}
                            }
                        }
                        Some(Err(e)) => { /* println!("Error: {:?}\r", e) */ } ,
                        None => break,
                    }
                }
            };
        }
    }

    fn is_quit(&self, e: KeyEvent) -> bool {
        e.modifiers == KeyModifiers::CONTROL && e.code == KeyCode::Char('q')
    }

    fn resize(&mut self, w: usize, h: usize) {
        if w != self.width {
            self.width = w;
        }
        if h != self.height {
            self.height = h;
        }
        self.upd = true;
        self.process.update_true();

        self.tree_view.set_height(self.height);
    }

    async fn handle_keyboard(&mut self, event: KeyEvent) {

        if self.is_lp_focused {
            self.handle_left_panel(event).await;
            return;
        }

        if event.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) {
            if event.code == KeyCode::Up {
                self.move_line_up().await;
            }
            if event.code == KeyCode::Down {
                self.move_line_down().await;
            }
            
            return;
        }

        match event.modifiers {
            KeyModifiers::ALT => {
                let code = event.code;
                match code {
                    KeyCode::Up => {
                        self.select_more();
                    }
                    KeyCode::Down => {
                        self.select_less();
                    }
                    // option + arrow left
                    KeyCode::Left =>  {
                        // scroll horizontal
                        // if self.x > 0 { self.x -= 1 }
                        let line = self.code.line_at(self.r);
                        if line.is_none() { return; }
                        let line = line.unwrap();
                        let next = utils::find_prev_word(line, self.c-1);
                        self.c = next;
                    },
                    // option + arrow right
                    KeyCode::Right => {
                        // scroll horizontal
                        // self.x += 1

                        let line = self.code.line_at(self.r);
                        if line.is_none() { return; }
                        let line = line.unwrap();
                        let next = utils::find_next_word(line, self.c+1);
                        self.c = next;
                    },
                    KeyCode::Backspace => self.handle_cut_line().await,

                    _ => debug!("event.code {:?}", event.code),
                }
                self.upd = true;
                return;
            }

            KeyModifiers::CONTROL => {
                match event.code {
                    KeyCode::Char('s') => self.save(),
                    KeyCode::Char('c') => self.copy_to_clipboard(),
                    KeyCode::Char('v') => self.paste_from_clipboard().await,
                    KeyCode::Char('d') => self.handle_duplicate().await,
                    KeyCode::Char('f') => self.local_search().await,
                    KeyCode::Char('r') => self.references().await,
                    KeyCode::Char('g') => self.definition().await,
                    KeyCode::Char('z') => self.undo().await,
                    KeyCode::Char('o') => self.undo_cursor().await,
                    KeyCode::Char('p') => self.redo_cursor().await,
                    KeyCode::Char('g') => {
                        self.global_search().await;
                        self.overlay_lines.clear();
                    },
                    KeyCode::Char('e') => self.handle_errors().await,
                    KeyCode::Char('h') => self.hover().await,
                    KeyCode::Char('t') => {
                        if self.lp_width == 0 { self.is_lp_focused = true; self.left_panel_toggle(); }
                        else {
                            if !self.is_lp_focused {
                                self.is_lp_focused = true;
                            } else {
                                self.is_lp_focused = false;
                                self.left_panel_toggle();
                            }
                        }


                        self.tree_view.upd = true;
                        self.upd = true;
                    }
                    KeyCode::Char('x') => {
                        self.copy_to_clipboard();
                        self.handle_cut().await;
                    }
                    KeyCode::Char(' ') => {
                        self.lsp_completion().await;
                    }
                    _ => {}
                }

                return;
            }

            KeyModifiers::SHIFT => match event.code {
                KeyCode::Char(_) => {}
                _ => {
                    if !self.selection.active && !self.selection.keep_once {
                        self.selection.set_start(self.r, self.c);
                    }

                    match event.code {
                        KeyCode::Up => self.handle_up(),
                        KeyCode::Down => self.handle_down(),
                        KeyCode::Left => self.handle_left(),
                        KeyCode::Right => self.handle_right(),
                        _ => {}
                    }

                    self.selection.set_end(self.r, self.c);
                    self.selection.active = true;
                    self.upd = true;
                    return;
                }
            },

            _ => {}
        }


        match event.code {
            KeyCode::Up => self.handle_up(),
            KeyCode::Down => self.handle_down(),
            KeyCode::Left => self.handle_left(),
            KeyCode::Right => self.handle_right(),
            KeyCode::Enter => self.handle_enter().await,
            KeyCode::Backspace => self.handle_delete().await,
            KeyCode::Char('÷') => self.comment_line().await,
            KeyCode::Char(c) => self.insert_char(c).await,
            KeyCode::Tab => self.insert_tab().await,
            _ => {
                debug!("event.code {:?}", event.code);
            }
        }


        if self.selection.active || self.selection.keep_once  {
            self.selection.clean();
            self.selection.keep_once = false;
            self.upd = true;
        }
    }


    async fn handle_left_panel(&mut self, event: KeyEvent) {
        match event.modifiers {
            KeyModifiers::CONTROL => {
                if event.code == KeyCode::Char('t') {
                    // close left panel 
                    self.is_lp_focused = false;
                    self.left_panel_toggle();
                    self.tree_view.upd = true;
                    self.upd = true;
                }
                return;
            }
            KeyModifiers::NONE => {

            },
            _ => {
                return;
            },
        }


        match event.code {
            KeyCode::Up => self.tree_view.handle_up(),
            KeyCode::Down => self.tree_view.handle_down(),
            KeyCode::Left => self.tree_view.handle_left(),
            KeyCode::Right => self.tree_view.handle_right(),
            KeyCode::Esc => {
                self.tree_view.clear_search();
            }
            KeyCode::Backspace => {
                self.tree_view.remove_filter_char();
            },
            KeyCode::Char(c) => {
                self.tree_view.insert_filter_char(c);
            }
            KeyCode::Enter => {
                match self.tree_view.get_selected() {
                    None => {}, Some(node) => {
                        if node.is_file() {
                            let path = node.fullpath();
                            self.cursor_history.push(CursorPosition{
                                filename: self.code.abs_path.clone(),
                                row: self.r, col: self.c, y: self.y, x: self.x,
                            });
                            self.tree_view.set_active(path.clone());
                            self.tree_view.clear_search();
                            self.tree_view.find_expand_by_fullpath(&path);
                            self.open_file(&path).await;
                            self.is_lp_focused = false;
                        }
                        else {
                            node.toggle();
                        }

                        self.upd = true;
                        self.tree_view.upd = true;
                    }
                }
            },
            _ => {
                debug!("event.code {:?}", event.code);
            }
        }
    }
    
    async fn open_file(&mut self, path: &String) {
        if !self.codes.contains_key(path) { // move self.code code to codes buffer

            self.code.set_cursor_position(
                self.r.clone(), self.c.clone(), self.y.clone(), self.x.clone()
            );

            let current_code = std::mem::replace(&mut self.code, Code::new());

            self.codes.insert(current_code.abs_path.clone(), current_code);
            self.load_file(path);

            let lang = self.code.lang.clone();
            let lsp = self.lang2lsp.get(&lang);
            match lsp {
                Some(lsp) => {
                    let mut lsp = lsp.lock().await;
                    let file_content = self.code.text.to_string();
                    lsp.did_open(&self.code.lang, &path, &file_content);
                },
                None => {
                    self.init_new_lsp();
                },
            }

        } else {  // move from codes buffer to self.code

            let mut code = self.codes.remove(path).unwrap();
            let (r,c,y,x) = code.get_cursor_position();

            self.code.set_cursor_position(
                self.r.clone(), self.c.clone(), self.y.clone(), self.x.clone()
            );

            let oldcode = std::mem::replace(&mut self.code, code);
            self.codes.insert(oldcode.abs_path.clone(), oldcode);
            self.r = r; self.c = c; self.y = y; self.x = x;
        }
    }

    async fn handle_mouse(&mut self, e: MouseEvent) {
        match e {
            MouseEvent { row, column, kind, modifiers } => {
                self.is_lp_focused = (column as usize) < self.lp_width;

                match (modifiers, kind) {
                    (KeyModifiers::CONTROL, MouseEventKind::Up(_)) => {
                        self.handle_mouse_click(row as usize, column as usize);
                        self.definition().await;
                        return;
                    },
                    (KeyModifiers::ALT, MouseEventKind::Up(_)) => {
                        self.handle_mouse_click(row as usize, column as usize);
                        self.references().await;
                        return;
                    },
                    _ => {}
                }

                match kind {
                    MouseEventKind::Down(button) => match button {
                        MouseButton::Left => {
                            let rrow = row as usize;
                            let ccol = column as usize;

                            if rrow == self.height-1 && (
                                (ccol == self.width - 9) ||
                                (ccol == self.width - 7) 
                            ){ 
                                // button clicked
                                return; 
                            }

                            if self.lp_width + self.ln_width < ccol &&
                                ccol < self.lp_width + self.ln_width + self.lns_width - 1 {
                                // clicked on run button column

                                return;
                            }

                            if (column as usize) + 1 == self.lp_width {
                                self.tree_view.set_moving(true);
                                self.upd = true;
                                return;
                            }

                            if self.is_lp_focused {
                                let maybe_node = self.tree_view.find(row as usize);

                                match maybe_node {
                                    Some(node) => {
                                        if node.is_file() {
                                            let path = node.fullpath();
                                            self.cursor_history.push(CursorPosition{
                                                filename: self.code.abs_path.clone(),
                                                row: self.r, col: self.c, y: self.y, x: self.x,
                                            });
                                            self.tree_view.set_active(path.clone());
                                            self.open_file(&path).await
                                        }
                                        else {
                                            node.toggle();
                                        }

                                        self.tree_view.set_selected(row as usize);
                                        self.upd = true;
                                        self.tree_view.upd = true;
                                    },
                                    None => {},
                                }
                                return;
                            }

                            let (prev_r, prev_c) = (self.r.clone(), self.c.clone());

                            self.handle_mouse_click(row as usize, column as usize);

                            if !self.selection.empty() {
                                self.selection.clean();
                                self.selection.set_start(self.r, self.c);
                                self.upd = true;
                            } else {
                                self.selection.set_start(self.r, self.c);
                                self.selection.active = true;
                                self.selection.keep_once = true;
                            }

                            if prev_r == self.r && prev_c == self.c && self.selection.empty() {
                                // double click
                                let line = self.code.line_at(self.r);
                                if line.is_none() { return; }
                                let line = line.unwrap();

                                let prev = utils::find_prev_word(line, self.c);
                                let next = utils::find_next_word(line, self.c);

                                if prev < self.c && self.c < next { // not first and last symbol
                                    self.selection.set_start(self.r, prev);
                                    self.selection.set_end(self.r, next);
                                    self.selection.active = true;
                                    self.selection.keep_once = true;
                                    self.upd = true;
                                }
                            }
                        }
                        MouseButton::Right => {}
                        MouseButton::Middle => {}
                    },
                    MouseEventKind::ScrollDown => {
                        if (column as usize) < self.lp_width {
                            self.tree_view.scroll_down();
                        } else {
                            self.scroll_down()
                        }
                    },
                    MouseEventKind::ScrollUp => {
                        if (column as usize) < self.lp_width {
                            self.tree_view.scroll_up();
                        } else {
                            self.scroll_up()
                        }
                    },
                    MouseEventKind::Up(_) => {
                        let rrow = row as usize;
                        let ccol = column as usize;

                        if rrow == self.height-1 && ccol == self.width - 9 {
                            // left panel button clicked
                            self.left_panel_toggle();
                            self.tree_view.upd = true;
                            self.upd = true;
                            return;
                        }
                        if rrow == self.height-1 && ccol == self.width - 7 {
                            // search button clicked
                            self.local_search().await;
                            return;
                        }

                        let is_runnable_button_clicked = self.lp_width + self.ln_width < ccol &&
                            ccol < self.lp_width + self.ln_width + self.lns_width -1;

                        if is_runnable_button_clicked {
                            match self.code.get_runnable(row as usize + self.y) {
                                Some(runnable) => self.process.start_tmux(&runnable.cmd),
                                None => {},
                            }
                            return;
                        }

                        self.cursor_history.push(CursorPosition{
                            filename: self.code.abs_path.clone(),
                            row: self.r, col: self.c, y: self.y, x: self.x,
                        });
                        self.cursor_history_undo.clear();


                        self.tree_view.set_moving(false);

                        if self.selection.active {
                            self.handle_mouse_click(row as usize, column as usize);

                            if self.selection.empty() {
                                self.selection.clean();
                                self.handle_movement();
                            } else {
                                self.selection.active = false;
                                self.selection.keep_once = true;
                                self.upd = true;
                            }
                        }
                    }
                    MouseEventKind::Drag(_) => {
                        if self.tree_view.is_moving() {
                            if column as usize > self.width - self.ln_width - self.lns_width {
                                return;
                            }
                            self.lp_width = column as usize;
                            self.tree_view.set_width(column as usize);
                            self.upd = true;
                            return;
                        }

                        self.handle_mouse_click(row as usize, column as usize);

                        self.selection.set_end(self.r, self.c);
                        self.selection.active = true;
                        self.selection.keep_once = true;
                        self.upd = true;
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_mouse_click(&mut self, row_click: usize, column_click: usize) {
        self.r = row_click + self.y;

        if self.r > self.code.len_lines() - 1 {  // fit to content
            self.r = self.code.len_lines() - 1;
        }

        if column_click < self.lp_width + self.ln_width + self.lns_width + self.x {
            self.c = 0; // outside of view
        } else {
            self.c = self.find_cursor_x_position(column_click - (self.lp_width + self.ln_width + self.lns_width));
            // self.c -= self.lp_width + self.ln_width + self.lns_width
            // self.c = column_click - self.lp_width -self.ln_width - self.lns_width + self.x;
        }

        if self.c > self.code.line_len(self.r) { // fit to content
            self.c = self.code.line_len(self.r);
        }
    }

    fn status_line(&self) -> String {
        if self.code.file_name.is_empty() {
            format!("  {} {} {} {} {}",
                '☰','☌', '', '▶', '⛭'
            )
        } else {
            let changed = if self.code.changed { "*" } else { " " };
            format!("  {}:{} {} {}{} {} {} {} {} {}",
                self.r + 1, self.c + 1, self.code.lang, self.code.file_name, changed,
                '☰','☌', '', '▶', '⛭'
            )
        }
    }

    fn clear_all(&mut self) {
        let mut stdout = stdout();
        queue!(stdout, terminal::Clear(ClearType::All)).unwrap();
        stdout.flush().expect("flush");
    }

    /*
       1. Get colors byte ranges from tree-sitter only for visible text
       2. Iterate over characters and increment bytes_counter
       3. Find color range that matches bytes_counter
       4. Draw each char

       According scrolling performance test:
       Colorization gives +5% cpu, 10 vs 15 % compared to no colors
       idea: adding colors to cache will improve performance of colored scrolling

       Filtering by row improves performance a bit, -2%
    */
    async fn draw(&mut self) {
        let start = time::Instant::now();

        if self.code.file_name.is_empty() {
            queue!(stdout(), cursor::Hide);
            if self.tree_view.is_search(){ queue!(stdout(), cursor::Show); }
            self.tree_view.draw();
            self.draw_logo();
            self.draw_status();
            self.tree_view.draw_search();
            stdout().flush().expect("flush");
            return;
        }

        self.tree_view.draw();
        self.draw_cursor();
        self.tree_view.draw_search();


        if !self.upd || self.height < 1 { return; } // it will do nothing if upd not marked


        let mut stdout = stdout();
        queue!(stdout, cursor::Hide).unwrap();


        let status = self.status_line();

        if self.width < self.lp_width + self.ln_width + self.lns_width + status.chars().count() { return; } // too small

        let colors = self.code.colors(self.y, self.y + self.height, &self.theme);

        let lines = self.code
            .slice(self.y, self.y + self.height)
            .lines()
            .take(self.height);

        let lines_count = lines.len();

        let mut bytes_counter = self.code.line_to_byte(self.y);

        let uri = format!("file://{}", self.code.abs_path.clone());

        let diagnostics = self.diagnostics.clone();
        let maybe_diagnostics = diagnostics.try_lock().unwrap();
        let maybe_diagnostics = maybe_diagnostics.get(&uri);

        let start_row = self.y.clone();
        let end_row = start_row + self.height;

        let line2error = match maybe_diagnostics {
            Some(d) =>  {
                d.diagnostics.iter()
                .filter(|d| d.severity == 1) // errors only
                .filter(|d| start_row <= d.range.start.line as usize  && d.range.start.line as usize <= end_row)
                .map(|d| (d.range.start.line as usize, &d.message))
                .collect::<HashMap<usize, &String>>()
            },
            None => HashMap::new(),
        };


        for (row, line) in lines.enumerate() {
            let rrow = row + self.y;

            queue!(stdout, cursor::MoveTo(self.lp_width as u16, row as u16)).unwrap();

            let line_number = format!("{:width$}", rrow + 1, width = self.ln_width);
            let lncolor = if line2error.contains_key(&rrow) { self.ecolor } else { self.lncolor };
            queue!(stdout, BColor(Color::Reset), FColor(lncolor), Print(line_number));

            let (run_or_empty, run_or_empty_color) = match self.code.is_runnable(rrow) {
                false => (" ".repeat(self.lns_width), Color::Reset),
                true => (format!("  {}  ", '▶'), Color::AnsiValue((87))), // todo: make it dynamic
            };

            // let (run_or_empty, run_or_empty_color) = (" ".repeat(self.lns_width), Color::Reset)

            queue!(stdout, BColor(Color::Reset), FColor(run_or_empty_color), Print(run_or_empty));
            queue!(stdout, BColor(Color::Reset), FColor(Color::Reset));


            let filtered_colors: Vec<_> = colors.iter()
                .filter(|(_, _, start, end, _)| *start <= rrow && rrow <= *end)
                .collect(); // todo:: add sort by range start and renge len

            let chars = line.chars();
            let chars_len = line.chars().len();
            let mut last_fg_color = Color::Reset;
            let mut last_bg_color = Color::Reset;
            let mut tabs_offset = 0;

            let is_overlayed = self.overlay_lines.contains(&row);

            for (col, ch) in chars.enumerate() {
                let outside_left = self.x > col;
                let outside_right = col >= self.width - self.lp_width - self.ln_width - self.lns_width  + self.x - 1 - tabs_offset;

                if outside_right || ch == '\n' || outside_left || is_overlayed {
                    bytes_counter += ch.len_utf8();
                    continue;
                }

                let color_range = filtered_colors.iter()
                    .find(|(start_byte, end_byte, _, _, _)| {
                        *start_byte <= bytes_counter && bytes_counter < *end_byte
                    });

                let fg_color = match color_range {
                    Some((_, _, _, _, color)) => *color,
                    None => Color::Reset,
                };

                let bg_color = match self.selection.is_selected(row + self.y, col) {
                    true => self.selcolor,
                    false => Color::Reset,
                };

                let chr = if ch == '\t' {
                    let tab_width = self.code.indent_width().unwrap_or(2);
                    if self.x == 0 { tabs_offset += tab_width; " ".repeat(tab_width) }
                    else { " ".to_string() }
                } else { ch.to_string() };

                if last_fg_color == fg_color && last_bg_color == bg_color {
                    queue!(stdout, Print(chr)).unwrap();
                } else {
                    queue!(stdout, BColor(bg_color), FColor(fg_color), Print(chr)).unwrap();
                    last_fg_color = fg_color;
                    last_bg_color = bg_color;
                }

                bytes_counter += ch.len_utf8();
            }


            if line2error.contains_key(&rrow) {
                let error_message = &line2error.get(&rrow).unwrap();
                self.draw_error(error_message, rrow, row)
            };

            // if row < self.height -1{
                queue!(stdout, BColor(Color::Reset), terminal::Clear(ClearType::UntilNewLine)).unwrap();
                // for some reason status line flickering effect
            // }

            if is_overlayed { continue; }

            // if row == self.height -1 && status.chars().count() < self.width {
            //     let x = self.lp_width + self.ln_width + self.lns_width +
            //          chars_len + line2error.get(&rrow).map(|e|e.len() +3).unwrap_or(0);

            //     let x1 = self.width -1 - status.chars().count();
            //     for c in x..=x1+1 { // for last line filling empty space manually until statusline
            //         queue!(stdout, BColor(Color::Reset), Print(' ')).unwrap();
            //     }
            // } else {
            //     queue!(stdout, BColor(Color::Reset), terminal::Clear(ClearType::UntilNewLine)).unwrap();
            // }
        }


        if lines_count < self.height && status.chars().count() < self.width {
            // queue!(stdout, terminal::Clear(ClearType::FromCursorDown)).unwrap(); // flickering???
            // fill empty space
            for row in lines_count..self.height {
                queue!(stdout, cursor::MoveTo(self.lp_width as u16, row as u16));
                queue!(stdout, BColor(Color::Reset), terminal::Clear(ClearType::UntilNewLine)).unwrap();
            }

            queue!(stdout, cursor::MoveTo(self.lp_width as u16, self.height as u16));
            for c in self.lp_width..self.width-status.chars().count()-1 {
                queue!(stdout, Print(' ')).unwrap();
            }
        }

        self.draw_status();
        self.draw_cursor();

        self.tree_view.draw_search();

        stdout.flush().expect("flush");

        // let elapsed = time::Instant::now() - start;
        // let ttr = format!("{:?} ns", elapsed.as_nanos()); // time to render

        // queue!(
        //     stdout,
        //     cursor::MoveTo((self.width - 40) as u16, (self.height) as u16),
        //     FColor(self.lncolor),
        //     Print(ttr),
        // )
        // .expect("Can not draw time to render");
        // self.draw_cursor();

        // stdout.flush().expect("flush");

        self.upd = false;
    }

    fn draw_error(&self, error_message: &String, rrow:usize, row:usize) {
        let space = 3;
        let max_x = self.lp_width + self.ln_width + self.lns_width + self.code.line_len(rrow) + space;

        if max_x > self.width { return; }

        queue!(stdout(), Print(" ".repeat(space)));

        let limit = self.width - max_x;

        let m: String = error_message.chars()
            .map(|ch| if ch == '\n' { ' ' } else { ch })
            .take(limit).collect();

        queue!(stdout(),
            cursor::MoveTo(max_x as u16, row as u16),
            BColor(Color::Reset),
            FColor(self.ecolor), Print(m)
        ).unwrap();
    }

    fn draw_cursor(&mut self) {
        if !self.cursor_is_focused() { return; }
        if self.code.file_name.is_empty() { return; }

        let out_left = self.c < self.x;
        let out_right = self.lp_width + self.ln_width + self.lns_width + self.c - self.x >= self.width;
        if out_left || out_right {
            queue!(stdout(), cursor::Hide).expect("Can not hide cursor");
            return;
        }

        let cursor_x_pos = if self.x != 0 { // if horizontal scroll, ignore indentation
            self.c + self.lp_width + self.ln_width + self.lns_width - self.x
        } else {
            let tabs_count = self.code.count_tabs(self.r, self.c).unwrap_or(0);
            let ident_width = self.code.indent_width().unwrap_or(2);
            let tabs_correction = tabs_count * (ident_width-1);
            self.c + self.lp_width + self.ln_width + self.lns_width - self.x + tabs_correction
        };

        let cursor_y_pos = self.r - self.y;

        queue!(
            stdout(),
            cursor::MoveTo(cursor_x_pos as u16, cursor_y_pos as u16),
            FColor(Color::Reset),
            cursor::Show
        )
        .expect("Can not show cursor");

        stdout().flush().expect("flush");
    }

    fn draw_status(&mut self) {
        let status = self.status_line();
        let x = self.width - status.chars().count();
        let y = self.height - 1;

        queue!(
            stdout(),
            cursor::Hide,
            cursor::MoveTo(x as u16, y as u16),
            FColor(self.scolor),
            Print(status)
        )
        .expect("Can not print status");

        stdout().flush().expect("flush");
    }

    fn draw_logo(&mut self) {
        let logo = r#"🅡 🅔 🅓"#;
        // let logo = "RED";

        let lines:Vec<&str> = logo.split("\n").collect();
        let logo_width = lines.get(0).unwrap().len();

        let fromy = self.height / 2 - lines.len() / 2;
        let fromx = self.lp_width + (self.width - self.lp_width)/ 2;

        let mut stdout = stdout();

        for r in 0..self.height{
            queue!(stdout,
                cursor::MoveTo(self.lp_width as u16, r as u16),
                terminal::Clear(ClearType::UntilNewLine)
            );
        }

        for (i,line) in lines.iter().enumerate() {
            queue!(stdout,
                cursor::MoveTo((fromx) as u16, (fromy + i) as u16),
                FColor(Color::Reset), Print(line)
            ).unwrap();
        }
    }

    fn find_cursor_x_position(&self, mx: usize) -> usize {
        let mut count = 0;
        let mut real_count = 0; // searching x position

        let line = self.code.get_line_at(self.r);
        if line.is_none() { return 0; }
        let line = line.unwrap();

        for ch in line.chars() {
            if count >= mx + self.x { break; }
            if ch == '\t' && self.x == 0 {
                count += self.code.indent_width().unwrap_or(2);
                real_count += 1;
            } else {
                count += 1;
                real_count += 1;
            }
        }

        real_count
    }


    fn cursor_is_focused(&mut self) -> bool {
        (self.r >= self.y) && (self.r - self.y) < self.height
    }
    fn cursor_is_invisible_at_bottom(&mut self) -> bool {
        self.r >= self.y && !self.cursor_is_focused()
    }
    fn cursor_is_invisible_at_top(&mut self) -> bool {
        self.y >= self.r && !self.cursor_is_focused()
    }
    fn cursor_is_invisible_at_left(&mut self) -> bool {
        self.c < self.x
    }
    fn cursor_is_invisible_at_right(&mut self) -> bool {
        self.lp_width + self.ln_width + self.lns_width + self.c - self.x >= self.width
    }

    fn focus_to_down(&mut self) {
        self.y = self.r - self.height + 1
    }
    fn focus_to_up(&mut self) {
        self.y = self.r
    }
    fn focus_to_right(&mut self) {
        self.x = self.c - self.width + 1 + self.ln_width + self.lns_width + self.lp_width;
    }
    fn focus_to_left(&mut self) {
        self.x = self.c;
    }
    fn focus_to_center(&mut self) {
        if self.r > self.height / 2 {
            self.y = self.r - (self.height / 2)
        }
    }
    fn fit_cursor(&mut self) {
        if self.c > self.code.line_len(self.r) {
            self.c = self.code.line_len(self.r)
        }
    }

    fn handle_up(&mut self) {
        if self.r > 0 {
            self.r -= 1;
            self.fit_cursor();
            self.handle_movement();
        }
    }

    fn handle_down(&mut self) {
        if self.r < self.code.len_lines() - 1 {
            self.r += 1;
            self.fit_cursor();
            self.handle_movement();
        }
    }

    fn handle_left(&mut self) {
        if self.c > 0 {
            self.c -= 1;
            if self.x > 0 && self.cursor_is_invisible_at_left() {
                self.focus_to_left();
                self.upd = true
            }
            if self.cursor_is_invisible_at_right() {
                self.focus_to_right();
                self.upd = true
            }
        } else if self.r > 0 {
            self.r -= 1;
            self.c = self.code.line_len(self.r);
        }

        self.handle_movement();
    }

    fn handle_right(&mut self) {
        if self.c < self.code.line_len(self.r) {
            self.c += 1;
            if self.x > 0 && self.cursor_is_invisible_at_left() {
                self.focus_to_left();
                self.upd = true
            }
            if self.cursor_is_invisible_at_right() {
                self.focus_to_right();
                self.upd = true
            }
        } else if self.r < self.code.len_lines() - 1 {
            self.r += 1;
            self.c = 0;
        }

        self.handle_movement();
    }

    async fn handle_enter(&mut self) {
        let ic = self.code.indentation_level(self.r);

        self.insert_char('\n').await;

        self.upd = true;
        self.r += 1;
        self.c = 0;

        match self.code.indent_string() {
            Some(indent_string) => {
                let indentation = indent_string.repeat(ic);
                self.code.insert_text(&indentation, self.r, self.c);

                if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                    lsp.lock().await.did_change(
                        self.r, self.c, self.r, self.c,
                        &self.code.abs_path, &indentation
                    ).await;
                }

                self.clean_diagnostics();

                self.c = indentation.chars().count();
            },
            None => {},
        }
        self.handle_movement();
    }

    async fn handle_delete(&mut self) {
        if self.selection.non_empty_and_active() {
            // remove selected text
            self.handle_cut().await;
            return;
        }

        if self.code.indent_unit().is_some() && self.c != 0
            && self.code.is_only_indentation_before(self.r, self.c)
            && self.code.indentation_level(self.r) > 0 {
            // remove indentations only

            let remove_all_indents = true;

            let il = self.code.indentation_level(self.r);
            let mut indent_from = match self.code.indent_unit() { // vscode like removal
                Some(unit) if unit == "\t" =>  il-1,
                Some(unit) if unit == " " => {
                    let w = self.code.indent_width().unwrap_or(2);
                    w * (il-1)
                },
                _ => self.c - 1,
            };

            if remove_all_indents { indent_from = 0 }  // idea like removal

            self.code.remove_text(self.r, indent_from, self.r, self.c);

            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                lsp.lock().await.did_change(
                    self.r, indent_from, self.r, self.c,
                    &self.code.abs_path, ""
                ).await;
            }

            self.c = indent_from;
            self.upd = true;
            self.clean_diagnostics();

            if remove_all_indents == false { return }
        }

        if self.c > 0 {
            // remove single char
            self.code.remove_char(self.r, self.c);

            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                lsp.lock().await.did_change(self.r, self.c-1, self.r, self.c, &self.code.abs_path, "").await;
            }

            self.c -= 1;
            self.upd = true;
            self.clean_diagnostics();

        } else if self.r != 0 {
            // remove enter char
            let prev_line_len = self.code.line_len(self.r - 1);
            // self.code.remove_char(self.r, self.c);
            self.code.remove_text(self.r - 1, prev_line_len, self.r, self.c);

            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                lsp.lock().await.did_change(self.r - 1, prev_line_len, self.r, self.c, &self.code.abs_path, "").await;
            }

            self.r -= 1;
            self.c = prev_line_len;
            self.upd = true;

            self.clean_diagnostics();
        }

        self.handle_movement();
    }

    fn copy_to_clipboard(&mut self) {
        if self.selection.empty() { return; }

        let (y, x) = self.selection.from();
        let (yto, xto) = self.selection.to();
        let text = self.code.get_text(y, x, yto, xto);

        let mut ctx = ClipboardContext::new().unwrap();
        ctx.set_contents(text).unwrap();
        // let mut clipboard = arboard::Clipboard::new().unwrap();
        // clipboard.set_text(text).unwrap();
    }

    async fn paste_from_clipboard(&mut self) {
        if self.selection.non_empty_and_active() {
            self.handle_cut().await;
        }

        // let mut clipboard = arboard::Clipboard::new().unwrap();  // slow comp time because of images lib
        // let text = clipboard.get_text().unwrap_or_default();


        let mut ctx = ClipboardContext::new().unwrap();
        let text = ctx.get_contents().unwrap();
        self.code.insert_text(&text, self.r, self.c);

        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        if let Some(lsp) = self.lang2lsp.get(lang) {
            lsp.lock().await.did_change(self.r, self.c, self.r, self.c, &path, &text).await;
        }

        self.clean_diagnostics();

        for ch in text.chars() {
            match ch {
                '\n' => {
                    self.r += 1;
                    self.c = 0;
                }
                _ => self.c += 1,
            }
        }

        self.upd = true;
    }

    fn selected_text(&mut self) -> String {
        let (y, x) = self.selection.from();
        let (yto, xto) = self.selection.to();
        let text = self.code.get_text(y, x, yto, xto);
        return text;
    }

    async fn handle_duplicate(&mut self) {
        if self.selection.non_empty_and_active() {
            let text = self.selected_text();
            self.code.insert_text(&text, self.r, self.c);

            let path = &self.code.abs_path;
            let lang = &self.code.lang;

            if let Some(lsp) = self.lang2lsp.get(lang) {
                lsp.lock().await.did_change(self.r, self.c, self.r, self.c, &path, &text).await;
            }

            for ch in text.chars() {
                match ch { // calculate cursor position
                    '\n' => { self.r += 1; self.c = 0; }
                    _ => self.c += 1,
                }
            }

            self.selection.clean();
            self.selection.keep_once = false;
            self.upd = true;
            self.clean_diagnostics();

        } else if self.r < self.code.len_lines() - 1 {
            let text = self.code.get_text(self.r, 0, self.r + 1, 0);
            self.r += 1;
            self.code.insert_text(&text, self.r, 0);

            let path = &self.code.abs_path;
            let lang = &self.code.lang;

            if let Some(lsp) = self.lang2lsp.get(lang) {
                let change_text = format!("\n{}", &text);
                lsp.lock().await
                    .did_change(self.r-1, text.len(), self.r-1, text.len(), path, &change_text)
                    .await;
            }

            self.upd = true;
            self.clean_diagnostics();
        }
    }

    async fn handle_cut(&mut self) {
        if self.selection.empty() { return; }

        let (y, x) = self.selection.from();
        let (yto, xto) = self.selection.to();
        self.code.remove_text(y, x, yto, xto);

        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        if let Some(lsp) = self.lang2lsp.get(lang) {
            lsp.lock().await.did_change(y, x, yto, xto, path, "").await;
        }

        self.r = y;
        self.c = x;
        self.selection.clean();
        self.selection.keep_once = false;
        self.upd = true;
        self.clean_diagnostics();
    }

    async fn handle_cut_line(&mut self) {
        self.code.remove_text(self.r, 0, self.r + 1, 0);

        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        if let Some(lsp) = self.lang2lsp.get(lang) {
            lsp.lock().await.did_change(self.r, 0, self.r + 1, 0,  path, "").await;
        }

        if self.c > self.code.line_len(self.r) { // fit to line
            self.c = self.code.line_len(self.r);
        }

        self.selection.clean();
        self.selection.keep_once = false;
        self.upd = true;
        self.clean_diagnostics();
    }

    fn handle_movement(&mut self) {
        if self.cursor_is_focused() {
            // optimization
            self.draw_status(); // no need full update
            self.draw_cursor();
            return;
        }
        if self.cursor_is_invisible_at_bottom() {
            self.upd = true; // needs full update
            self.focus_to_down();
            return;
        }
        if self.cursor_is_invisible_at_top() {
            self.upd = true; // needs full update
            self.focus_to_up();
            return;
        }
    }

    fn scroll_down(&mut self) {
        if self.y + self.height >= self.code.len_lines() {
            return;
        }
        self.y += 1;
        self.upd = true;
    }
    fn scroll_up(&mut self) {
        if self.y == 0 {
            return;
        }
        self.y -= 1;
        self.upd = true;
    }

    async fn insert_char(&mut self, c: char) {
        if self.selection.non_empty_and_active() { self.handle_cut().await;}

        self.code.insert_char(c, self.r, self.c);

        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        if let Some(lsp) = self.lang2lsp.get(lang) {
            lsp.lock().await.did_change(self.r, self.c, self.r, self.c, path, &c.to_string()).await;
        }

        self.c += 1;
        self.upd = true;
        self.clean_diagnostics();
    }

    async fn insert_tab(&mut self) {
        let (r,c) = (self.r, self.c);
        let inserted = self.code.insert_tab(r,c);

        self.c += inserted.chars().count();

        if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
            lsp.lock().await.did_change(r,c, r,c, &self.code.abs_path, &inserted).await;
        }
        self.upd = true;
        self.clean_diagnostics();
    }

    async fn comment_line(&mut self) {
        let comment = self.code.get_lang_comment();
        if comment.is_none() { return; }
        let comment = comment.unwrap();

        match self.code.find_comment(self.r) {
            Some(comment_index) => {  // uncomment line
                let comment_len = comment.len();
                self.code.remove_text(self.r, comment_index, self.r, comment_index + comment_len);

                let path = &self.code.abs_path; let lang = &self.code.lang;

                if let Some(lsp) = self.lang2lsp.get(lang) {
                    lsp.lock().await.did_change(
                        self.r, comment_index,
                        self.r, comment_index + comment_len,
                        path, ""
                    ).await;
                }

            },
            None => {  // comment line
                let first_non_whitespace = self.code
                    .find_first_non_whitespace(self.r, self.c)
                    .unwrap_or(0);

                self.code.insert_text(&comment, self.r, first_non_whitespace);

                let path = &self.code.abs_path; let lang = &self.code.lang;

                if let Some(lsp) = self.lang2lsp.get(lang) {
                    lsp.lock().await.did_change(
                        self.r, first_non_whitespace,
                        self.r, first_non_whitespace,
                        &path, &comment
                    ).await;
                }
            },
        }

        self.upd = true;
        self.handle_down();
    }

    fn save(&mut self) {
        self.code.save_file().expect("Can not save file");
        self.upd = true;
    }

    async fn undo(&mut self) {
        let maybe_change = self.code.undo();
        match maybe_change {
            Some(changes) => {

                for change in changes.changes {

                    self.r = change.row;
                    self.c = change.column;
                    let text = &change.text;

                    match change.operation {
                        crate::code::Operation::Insert => {
                            let r = change.row;
                            let c = change.column;
                            let mut r_end = r;
                            let mut c_end = c;

                            for ch in text.chars() { match ch {
                                '\n' => { r_end += 1; c_end = 0;}
                                _ => c_end += 1,
                            }}

                            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                                lsp.lock().await.did_change(
                                    r, c, r_end, c_end, &self.code.abs_path, ""
                                ).await;
                            }
                        },
                        crate::code::Operation::Remove => {
                            let mut r = change.row;
                            let mut c = change.column;

                            for ch in text.chars() { match ch {
                                '\n' => { r -= 1; c = 0;}
                                _ => c -= 1,
                            }}
                            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                                lsp.lock().await.did_change(
                                    r, c,  r, c, &self.code.abs_path, &change.text
                                ).await;
                            }
                        }
                        crate::code::Operation::Start => {},
                        crate::code::Operation::End => {},
                    }

                }
            },
            None => {},
        }
        self.upd = true;
    }
    async fn local_search(&mut self) {
        let mut end = false;
        let mut changed = false;

        self.search.active = true;

        if self.selection.non_empty_and_active() {
            let (y, x) = self.selection.from();
            let (yto, xto) = self.selection.to();
            let selected_text = self.code.get_text(y, x, yto, xto);
            self.search.pattern = ropey::Rope::from_str(&selected_text);
            changed = true;
        }
        if self.search.pattern.len_chars() > 0 {
            let search_results = self.code.search(&self.search.pattern.to_string());
            let search_results: Vec<SearchResult> = search_results.iter()
                .map(|(line, position)| SearchResult{ line:*line, position:*position })
                .collect();
            self.search.results = search_results;
            self.search.index = 0;
            changed = true;
        }

        let mut x = self.search.pattern.len_chars();

        while !end {

            self.draw_search_line(x, self.height-1);

            if changed && self.search.pattern.len_chars() > 0 &&
                !self.search.results.is_empty() {

                let search_result = &self.search.results[self.search.index];
                let sy = search_result.line;
                let sx = search_result.position;
                self.r = sy;
                self.c = sx + self.search.pattern.chars().count();
                self.handle_movement();
                if self.r - self.y == self.height-1 { self.y += 1; }  // if last line, scroll down
                self.selection.active = true;
                self.selection.set_start(sy, sx);
                self.selection.set_end(sy, sx + self.search.pattern.chars().count());

                self.upd = true;
                self.draw().await;
                self.draw_search_line(x, self.height-1);

                changed = false;
            }

            let mut reader = EventStream::new();
            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            changed = false;

                            match event {
                                Event::Key(event) => {
                                    match (event.modifiers, event.code) {
                                        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
                                            self.global_search().await;
                                            self.overlay_lines.clear();
                                            return;
                                        }
                                        _ => {}
                                    }

                                    match event.code {
                                        KeyCode::Up => {
                                            if self.search.index > 0 {
                                                self.search.index -= 1;
                                            } else {
                                                self.search.index = self.search.results.len() - 1
                                            }
                                            changed = true;
                                        },
                                        KeyCode::Down => {
                                            self.search.index += 1;
                                            if self.search.index >= self.search.results.len() {
                                                self.search.index = 0;
                                            }
                                            changed = true;
                                        },
                                        KeyCode::Left if x > 0 => x -= 1,
                                        KeyCode::Right if x < self.search.pattern.len_chars() => x += 1,
                                        KeyCode::Enter => { end = true; },
                                        KeyCode::Esc => { end = true; },
                                        KeyCode::Backspace if x > 0 => {
                                            x -= 1;
                                            self.clean_search_line();
                                            self.search.pattern.remove(x..x+1);

                                            if self.search.pattern.len_chars() > 0 {

                                                let search_results = self.code.search(&self.search.pattern.to_string());
                                                let search_results: Vec<SearchResult> = search_results.iter()
                                                    .map(|(line, position)| SearchResult{ line:*line, position:*position })
                                                    .collect();

                                                self.search.results = search_results;
                                                self.search.index = 0;
                                                changed = true;
                                            }
                                        },
                                        KeyCode::Char(c) => {
                                            self.clean_search_line();
                                            self.search.pattern.insert_char(x, c);
                                            x += 1;
                                            let search_results = self.code.search(&self.search.pattern.to_string());
                                            let search_results: Vec<SearchResult> = search_results.iter()
                                                .map(|(line, position)| SearchResult{ line:*line, position:*position })
                                                .collect();

                                            self.search.results = search_results;
                                            self.search.index = 0;
                                            changed = true;
                                            // debug!("search_results {:?}", search_results);
                                        },
                                        _ => {
                                            debug!("event.code {:?}", event.code);
                                        }
                                    }
                                }
                                _ => {}
                            }

                        }

                        Some(Err(e)) => {
                            debug!("Error: {:?}\r", e);
                            end = true;
                        },
                        None => { end = true; },
                    }
                }
            };
        }
        self.upd = true;
        self.search.active = false;
    }
    pub fn draw_search_line(&mut self, x:usize, y:usize) {
        let prefix = "search: ";
        let line = if !self.search.results.is_empty() && self.search.pattern.len_chars() > 0 {
            let postfix = format!("{}/{}", self.search.index+1, self.search.results.len());
            format!("{}{} {}", prefix, &self.search.pattern, postfix)
        } else {
            format!("{}{} ", prefix, &self.search.pattern)
        };

        queue!(stdout(),
            cursor::MoveTo((self.lp_width + 1) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(line),
        );
        queue!(stdout(),
            cursor::MoveTo((self.lp_width + 1 + prefix.len() + x) as u16, y as u16),
        );

        stdout().flush();
    }
    pub fn clean_search_line(&mut self) {
        let prefix = "search: ";
        let line = if !self.search.results.is_empty() && self.search.pattern.len_chars() > 0 {
            let postfix = format!("{}/{}", self.search.index+1, self.search.results.len());
            format!("{}{} {}", prefix, &self.search.pattern, postfix)
        } else {
            format!("{}{}", prefix, &self.search.pattern)
        };

        queue!(stdout(),
            cursor::MoveTo((self.lp_width + 1) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(" ".repeat(line.chars().count())),
        );

        stdout().flush();
    }

    pub fn init_new_lsp(&mut self) {
        let lang = self.code.lang.clone();
        if self.lang2lsp.contains_key(&lang) { return; }

        let lsp = Arc::new(Mutex::new(lsp::Lsp::new()));
        let lsp_clone = lsp.clone();

        let abs_file = self.code.abs_path.clone();
        let file_content = self.code.text.to_string();

        self.lang2lsp.insert(lang.clone(), lsp_clone);
        let diagnostic_send = self.diagnostics_sender.as_mut().map(|s|s.clone()).unwrap();

        let lsp_cmd = self.code.get_lang_conf()
            .map(|c| c.lsp.clone())
            .flatten()
            .map(|l| l.join(" "));

        if lsp_cmd.is_none() { return; }

        let lsp_cmd = lsp_cmd.unwrap();

        tokio::task::spawn(async move {
            // lsp start, initialization
            let mut lsp = lsp.lock().await;

            let result = lsp.start(&lang, &lsp_cmd, Some(diagnostic_send));

            match result {
                Ok(_) => {},
                Err(e) => {
                    debug!("error starting lsp process {}", e.to_string());
                    return;
                },
            }

            let dir = utils::current_dir();
            lsp.init(&dir).await;

            lsp.did_open(&lang, &abs_file, &file_content);
        });
    }

    pub fn clean_diagnostics(&mut self) {
        let uri = format!("file://{}", self.code.abs_path);
        self.diagnostics.clone().try_lock().unwrap().remove(&uri);
    }

    pub async fn set_lsp_status(&mut self, status: &str) {
        let lsp_status = self.lsp_status.clone();
        let mut lsp_status = lsp_status.lock().await;
        *lsp_status = status.to_string();
    }


    fn score_matches(src: &str, match_str: &str) -> i32 {
        let mut score = 0;

        // If the match is at the beginning, we give it a high score.
        if src.starts_with(match_str) {
            score += 1000;
        }

        // Each occurrence of match_str in src adds a smaller score.
        score += (src.matches(match_str).count() as i32) * 10;

        // If match is close to the start of the string but not at the beginning, add some score.
        if let Some(initial_index) = src.find(match_str) {
            if initial_index > 0 && initial_index < 5 {
                score += 500;
            }
        }

        score
    }

    fn sort_completion_items(&self, items: &mut Vec<CompletionItem>, prev_word: &str) {
        items.sort_by(|a, b| {
            let sa = Self::score_matches(&a.label, prev_word);
            let sb = Self::score_matches(&b.label, prev_word);
            let r = sb.cmp(&sa);
            if r == Ordering::Equal {
                a.label.len().cmp(&b.label.len())
            } else {
                r
            }
        });
    }
    pub async fn lsp_completion(&mut self) {
        let mut end = false;

        while !end {
            let mut changed = false;

            let path = &self.code.abs_path;
            let lang = &self.code.lang;

            let completion_result = match self.lang2lsp.get(lang) {
                Some(lsp) => lsp.lock().await.completion(&path, self.r, self.c).await,
                None => return,
            };

            let mut completion_result = match completion_result {
                Some(c) => c, None => return,
            };

            if completion_result.items.is_empty() { return; }

            self.set_lsp_status("lsp completion").await;

            let (mut selected, mut selected_offset) = (0, 0);
            let (height, mut width) = (5, 30);

            let line = match self.code.line_at(self.r) {
                Some(line) => line, None => return,
            };

            let prev = utils::find_prev_word(line, self.c);
            let prev_word = line.chars().skip(prev).take(self.c - prev).collect::<String>();

            // Sort completion items
            self.sort_completion_items(&mut completion_result.items, &prev_word);

            let mut options = &completion_result.items;

            while !changed {

                let mut reader = EventStream::new();


                // calculate scrolling offsets
                if selected < selected_offset { selected_offset = selected }
                if selected >= selected_offset + height { selected_offset = selected - height + 1 }

                self.lsp_completion_draw(height, width, options, selected, selected_offset);
                self.upd_next = true;

                let mut event = reader.next().fuse();

                select! {
                    maybe_event = event => {
                        changed = false;
                        match maybe_event {
                            Some(Ok(event)) => {
                                if event == Event::Key(KeyCode::Esc.into()) { self.upd = true; return ;}
                                if event == Event::Key(KeyCode::Down.into()) && selected < options.len() - 1 { selected += 1;}
                                if event == Event::Key(KeyCode::Up.into()) && selected > 0 { selected -= 1; }
                                if event == Event::Key(KeyCode::Enter.into())
                                    || event == Event::Key(KeyCode::Tab.into())
                                {
                                    let item = completion_result.items.get(selected).unwrap();
                                    self.lsp_completion_apply(item).await;
                                    return;
                                }


                                if event == Event::Key(KeyCode::Left.into()) {
                                    changed = true;
                                    self.handle_left();
                                    self.upd = true;
                                    self.draw().await;
                                }
                                if event == Event::Key(KeyCode::Right.into()) {
                                    changed = true;
                                    self.handle_right();
                                    self.upd = true;
                                    self.draw().await;
                                }
                                if event == Event::Key(KeyCode::Backspace.into()) {
                                    changed = true;
                                    self.handle_delete().await;
                                    self.upd = true;
                                    self.draw().await;
                                }
                                match event {
                                    Event::Key(event) => {
                                        match event.code {
                                            KeyCode::Char(' ') => { self.upd = true; return ;}
                                            KeyCode::Char(c) => {
                                                changed = true;
                                                self.insert_char(c).await;
                                                self.upd = true;
                                                self.draw().await;
                                            },
                                            _ => {},
                                        }
                                    },
                                    _ => {},
                                }
                                // KeyCode::Backspace => self.handle_delete().await,
                                // KeyCode::Char('÷') => self.comment_line().await,
                                // KeyCode::Char(c) => self.insert_char(c).await,

                            }
                            Some(Err(e)) => {debug!("Error: {:?}\r", e) },
                            None => break,
                        }
                    }
                };
            }
        }

    }

    pub fn lsp_completion_draw(&mut self,
        height: usize, width:usize,
        options: &Vec<CompletionItem>,
        selected:usize, offset:usize
    ) {
        let width = options.iter().map(|o| o.label.len()).max().unwrap_or(width);

        for row in 0..height {
            if row >= options.len() || row >= height { break; }
            let option = &options[row + offset];

            let is_selected = selected == row + offset;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let label = format!(" {:width$} ", option.label, width = width);

            queue!(stdout(),
                cursor::MoveTo(
                    (self.c + self.lp_width + self.ln_width + self.lns_width - 2) as u16,
                    (self.r - self.y + row + 1) as u16
                ),
                BColor(bgcolor), FColor(self.lncolor),
                Print(label),
                BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        stdout().flush().expect("cant flush");
        self.draw_cursor();
        stdout().flush().expect("cant flush");
    }

    pub async fn lsp_completion_apply(&mut self, item: &lsp::lsp_messages::CompletionItem) {
        if item.textEdit.is_none() && item.label.is_empty() { return; }

        let line = match self.code.line_at(self.r) {
            Some(line) => line, None => return,
        };

        let prev = utils::find_prev_word(line, self.c);
        let next = utils::find_next_word(line, self.c);

        let insert_text = match item.textEdit.as_ref() {
            Some(t) => &t.newText, None => &item.label,
        };

        self.code.remove_text(self.r, prev, self.r, next);
        self.code.insert_text(insert_text, self.r, prev);

        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        if let Some(lsp) = self.lang2lsp.get(lang) {
            lsp.lock().await.did_change(self.r, prev, self.r, next,  &path, "").await;
            lsp.lock().await.did_change(self.r, prev, self.r, prev, &path, insert_text).await;
        }

        self.c = prev + insert_text.len();
        self.upd = true;
        self.clean_diagnostics();
    }

    async fn definition(&mut self) {
        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        let definition_result = match self.lang2lsp.get(lang) {
            Some(lsp) => lsp.lock().await.definition(&path, self.r, self.c).await,
            None => { return; },
        };

        let definition = match &definition_result {
            Some(def) if def.len() == 1 => &def[0],
            _ => return,
        };

        if definition.uri != format!("file://{}", self.code.abs_path) {
            let path = definition.uri.split("file://").nth(1).unwrap().to_string();
            self.open_file(&path).await;
        }

        if definition.range.start.line as usize > self.code.len_lines() ||
            definition.range.start.character as usize >
                self.code.line_len(definition.range.start.line as usize) {
            return;
        }

        self.r = definition.range.start.line as usize;
        self.c = definition.range.start.character as usize;
        self.handle_movement();

        self.cursor_history.push(CursorPosition{
            filename: self.code.abs_path.clone(),
            row: self.r, col: self.c, y: self.y, x: self.x,
        });
        self.cursor_history_undo.clear();

        self.upd = true;
        self.tree_view.upd = true;
    }

    pub async fn references(&mut self) {
        let (r, c, initial_abs_path) = (self.r.clone(), self.c.clone(), self.code.abs_path.clone());

        loop {
            let start = Instant::now();

            let references_result = match self.lang2lsp.get(&self.code.lang) {
                Some(lsp) => lsp.lock().await.references(&self.code.abs_path, self.r, self.c).await,
                None => return,
            };

            let elapsed = start.elapsed().as_millis();

            let references = match references_result {
                Some(refr) if !refr.is_empty() => refr,
                _ => return,
            };

            if references.len() == 0 { return; }
            if references.len() == 1 { self.apply_reference(&references[0]).await; return; }

            let max_visible = 3;
            let (mut selected, mut selected_offset) = (0, 0);
            let (height, mut width) = (max_visible, 30);
            self.upd = true; self.tree_view.upd = true;

            self.overlay_lines.clear();

            let mut reader = EventStream::new();

            loop {

                if selected < selected_offset { selected_offset = selected } // calculate scrolling offsets
                if selected >= selected_offset + height { selected_offset = selected - height + 1 }

                let reference = references.get(selected).unwrap();

                if reference.uri != format!("file://{}", &self.code.abs_path) {
                    let path = reference.uri.split("file://").nth(1).unwrap().to_string();
                    self.open_file(&path).await;
                }

                self.r = reference.range.start.line as usize;
                self.c = reference.range.start.character as usize;
                self.handle_movement();
                self.selection.set_start(reference.range.start.line as usize, reference.range.start.character as usize);
                self.selection.set_end(reference.range.end.line as usize, reference.range.end.character as usize);
                self.selection.activate();

                let count = std::cmp::min(max_visible, references.len());
                let fromy = self.height - count - 1;
                for i in fromy..=self.height { self.overlay_lines.insert(i); }

                self.draw().await;
                self.references_draw(height, width, fromy, &references, selected, selected_offset, elapsed);
                self.draw_cursor();

                let mut event = reader.next().fuse();

                select! {
                    maybe_event = event => {
                        match maybe_event {
                            Some(Ok(event)) => {
                                if event == Event::Key(KeyCode::Esc.into()) {
                                    if self.code.abs_path != initial_abs_path {
                                        self.open_file(&initial_abs_path).await;
                                    }
                                    self.r = r; self.c = c;
                                    self.handle_movement();
                                    self.selection.clean();

                                    self.upd = true; self.tree_view.upd = true;
                                    self.overlay_lines.clear();
                                    return;
                                }
                                if event == Event::Key(KeyCode::Down.into()) && selected < references.len() - 1 {
                                    selected += 1;
                                    self.upd = true;
                                    self.tree_view.upd = true;
                                }
                                if event == Event::Key(KeyCode::Up.into()) && selected > 0 {
                                    selected -= 1;
                                    self.upd = true; self.tree_view.upd = true;
                                }
                                if event == Event::Key(KeyCode::Enter.into())
                                || event == Event::Key(KeyCode::Tab.into()) {
                                    self.apply_reference(reference).await;
                                    self.overlay_lines.clear();
                                    return;
                                }
                            }
                            Some(Err(e)) => { debug!("Error: {:?}\r", e); self.overlay_lines.clear(); return; },
                            None => break,
                        }
                    }
                };
            }
        }
    }

    async fn apply_reference(&mut self, reference: &ReferencesResult) {
        if reference.uri != format!("file://{}", self.code.abs_path) {
            let path = reference.uri.split("file://").nth(1).unwrap().to_string();
            self.open_file(&path).await;
        }

        self.r = reference.range.start.line as usize;
        self.c = reference.range.start.character as usize;
        self.handle_movement();

        self.cursor_history.push(CursorPosition{
            filename: self.code.abs_path.clone(),
            row: self.r, col: self.c, y: self.y, x: self.x,
        });
        self.cursor_history_undo.clear();

        self.upd = true;
        self.tree_view.upd = true;
    }

    pub fn references_draw(&mut self,
        height: usize, width:usize, fromy:usize,
        options: &Vec<ReferencesResult>,
        selected: usize, offset: usize, elapsed:u128
    ) {
        let options: Vec<String> = options.iter().enumerate().map(|(i, reff)| {
            format!(
                "{}/{} {}:{} {}", i+1, options.len(), reff.uri.strip_prefix("file://").unwrap(),
                    reff.range.start.line, reff.range.start.character,
                )
        }).collect();

        let width = options.iter().map(|o| o.len()).max().unwrap_or(width);

        for row in 0..options.len() {
            if row >= options.len() || row >= height { break; }
            let option = &options[row + offset];

            let is_selected = selected == row + offset;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let label = format!(" {:width$} ", option, width = width);

            queue!(stdout(),
                cursor::MoveTo((self.lp_width + self.ln_width + self.lns_width - 2) as u16, (row + fromy) as u16),
                BColor(bgcolor), FColor(self.lncolor), Print(label),  BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        let status = format!("lsp references, elapsed {} ms", elapsed);

        queue!(stdout(),
            cursor::MoveTo((self.lp_width + self.ln_width + self.lns_width - 2) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(status),
        );

        stdout().flush().expect("cant flush");
    }

    pub async fn hover(&mut self) {
        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        let maybe_hover_result = match self.lang2lsp.get(lang) {
            Some(lsp) => lsp.lock().await.hover(&path, self.r, self.c).await,
            None => return,
        };

        let hover_result = match maybe_hover_result {
            Some(hr) => hr,
            None => return,
        };

        self.set_lsp_status("lsp completion").await;

        let (mut end, mut selected, mut selected_offset) = (false, 0, 0);
        let (height, mut width) = (10, 30);

        let mut reader = EventStream::new();

        while !end {
            // calculate scrolling offsets
            if selected < selected_offset { selected_offset = selected }
            if selected >= selected_offset + height { selected_offset = selected - height + 1 }

            let options:Vec<String> = hover_result.contents.value.split("\n").map(|s| s.to_string()).collect();

            if options.is_empty() { return; }

            self.hover_draw(height, width, &options, selected, selected_offset);
            self.upd_next = true;

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if event == Event::Key(KeyCode::Esc.into()) { self.upd = true; break; }
                            if event == Event::Key(KeyCode::Down.into()) && selected < options.len() - 1 { selected += 1;}
                            if event == Event::Key(KeyCode::Up.into()) && selected > 0 { selected -= 1; }
                            if event == Event::Key(KeyCode::Enter.into())
                                || event == Event::Key(KeyCode::Tab.into())
                            {
                                break
                            }
                        }
                        Some(Err(e)) => {
                            debug!("Error: {:?}\r", e)
                        },
                        None => break,
                    }
                }
            }
        }

    }

    pub fn hover_draw(&mut self,
        height: usize, width:usize,
        options: &Vec<String>,
        selected: usize, offset: usize
    ) {
        let width = options.iter().map(|o| o.len()).max().unwrap_or(width);

        for row in 0..height {
            if row >= options.len() || row >= height { break; }
            let option = &options[row + offset];

            let bgcolor = if selected == row + offset { Color::Grey } else { Color::Reset };

            let label = format!(" {:width$} ", option, width = width);

            queue!(stdout(),
                cursor::MoveTo(
                    (self.c + self.lp_width + self.ln_width + self.lns_width - 2) as u16,
                    (self.r - self.y + row + 1) as u16
                ),
                BColor(bgcolor), FColor(self.lncolor), Print(label),
            );
        }

        self.draw_cursor();
        stdout().flush().expect("cant flush");
    }

    pub async fn handle_errors(&mut self) {
        let (r, c, initial_abs_path) = (self.r.clone(), self.c.clone(), self.code.abs_path.clone());

        let (mut selected, mut selected_offset) = (0, 0);
        let (height, mut width) = (5, 30);
        self.upd = true; self.tree_view.upd = true;

        let uri = format!("file://{}", self.code.abs_path);

        let diagnostics:Vec<Diagnostic> = {
            let diagnostics = self.diagnostics.clone();
            let maybe_diagnostics = diagnostics.lock().await;
            let maybe_diagnostics = maybe_diagnostics.get(&uri);

            let diagnostics:Vec<Diagnostic> = match maybe_diagnostics {
                Some(d) => d.diagnostics.iter().filter(|d| d.severity == 1).map(|d|d.clone()).collect(),
                None => return,
            };

            if diagnostics.is_empty() { return }

            diagnostics
        };

        let mut reader = EventStream::new();

        loop {

            if selected < selected_offset { selected_offset = selected } // calculate scrolling offsets
            if selected >= selected_offset + height { selected_offset = selected - height + 1 }

            let diagnostic = match diagnostics.get(selected) {
                Some(d) => d, None => { break },
            };

            // if diagnostic.uri != format!("file://{}", &self.code.abs_path) {
            //     let path = diagnostic.uri.split("file://").nth(1).unwrap().to_string();
            //     self.open_file(&path).await;
            // }

            self.r = diagnostic.range.start.line as usize;
            self.c = diagnostic.range.start.character as usize;
            self.handle_movement();
            self.selection.set_start(diagnostic.range.start.line as usize, diagnostic.range.start.character as usize);
            self.selection.set_end(diagnostic.range.end.line as usize, diagnostic.range.end.character as usize);
            self.selection.activate();

            // self.draw().await;
            self.diagnostic_draw(height, width, &diagnostics, selected, selected_offset);
            self.draw_cursor();

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if event == Event::Key(KeyCode::Esc.into()) {
                                if self.code.abs_path != initial_abs_path {
                                    self.open_file(&initial_abs_path).await;
                                }
                                self.r = r; self.c = c;
                                self.handle_movement();
                                self.selection.clean();

                                self.upd = true; self.tree_view.upd = true;
                                return;
                            }
                            if event == Event::Key(KeyCode::Down.into()) && selected < diagnostics.len() - 1 {
                                selected += 1;
                                self.upd = true;
                                self.tree_view.upd = true;
                            }
                            if event == Event::Key(KeyCode::Up.into()) && selected > 0 {
                                selected -= 1;
                                self.upd = true; self.tree_view.upd = true;
                            }

                            if let Event::Resize(w, h) = event {
                                self.upd = true;
                                self.tree_view.upd = true;
                                self.resize(w as usize, h as usize);
                                self.draw().await;
                            }

                            if event == Event::Key(KeyCode::Enter.into())
                            || event == Event::Key(KeyCode::Tab.into()) {
                                // self.apply_reference(diagnostic).await;
                                return;
                            }
                        }
                        Some(Err(e)) => { debug!("Error: {:?}\r", e); return; },
                        None => break,
                    }
                }
            };
        }
    }

    pub fn diagnostic_draw(&mut self,
        height: usize, width:usize,
        options: &Vec<Diagnostic>,
        selected: usize, offset: usize
    ) {

        let limit = self.width - self.lp_width - self.ln_width - self.lns_width - 1;

        let options: Vec<String> = options.iter().enumerate().map(|(i, diagnostic)| {
            let prefix = format!("{}/{} {}:{} ", i+1, options.len(),
                diagnostic.range.start.line,
                diagnostic.range.start.character,
            );
            let message = diagnostic.message.chars().take(limit-prefix.len()).collect::<String>();
            format!("{}{}", prefix, message)
        }).collect();

        let width = options.iter().map(|o| o.len()).max().unwrap_or(width);

        for row in 0..options.len() {
            if row >= options.len() || row >= height { break; }
            let option = &options[row + offset];

            let is_selected = selected == row + offset;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let label = format!(" {:width$} ", option, width = width);

            queue!(stdout(),
                cursor::MoveTo((self.lp_width + self.ln_width + self.lns_width - 1) as u16, row  as u16),
                BColor(bgcolor), FColor(self.lncolor), Print(label),  BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        stdout().flush().expect("cant flush");
    }

    pub async fn global_search(&mut self) {
        if self.search.pattern.len_chars() == 0 { return; }

        let (r, c, initial_abs_path) = (self.r.clone(), self.c.clone(), self.code.abs_path.clone());
        let max_visible = 3;
        let mut changed = true;
        let (mut selected, mut selected_offset) = (0, 0);
        let (mut height, mut width) = (max_visible, 30);
        self.upd = true; self.tree_view.upd = true;

        self.overlay_lines.clear();

        let path = Path::new("./");
        let mut search_results:Vec<(String, search::search::SearchResult)> = Vec::new();

        let start = Instant::now();
        let search_resilts = search::search::search_in_directory(&path, &self.search.pattern.to_string());

        let elapsed = start.elapsed().as_millis();

        match search_resilts {
            Ok(srs) => {
                for sr in srs {
                    for r in sr.search_results {
                        search_results.push((sr.file_path.clone(), r));
                    }
                }
            },
            Err(_) => return,
        }

        if search_results.is_empty() { return; }

        if search_results.len() < height { height = search_results.len() }

        let mut reader = EventStream::new();

        loop {

            if selected < selected_offset { selected_offset = selected } // calculate scrolling offsets
            if selected >= selected_offset + height { selected_offset = selected - height + 1 }

            if changed {
                let search_result = search_results.get(selected).unwrap();

                if search_result.0 != self.code.abs_path {
                    self.open_file(&search_result.0).await;
                }

                self.r = search_result.1.line-1;
                self.c = search_result.1.column;
                self.handle_movement();
                self.focus_to_center();
                self.selection.set_start(search_result.1.line-1, search_result.1.column);
                self.selection.set_end(search_result.1.line-1, search_result.1.column + self.search.pattern.chars().count());
                self.selection.activate();

                let fromy = self.height - std::cmp::min(max_visible, search_results.len());
                for i in fromy-1..=self.height { self.overlay_lines.insert(i); }

                self.draw().await;
                self.draw_search_result(height, width, fromy-1, &search_results, selected, selected_offset, elapsed);
                self.draw_cursor();
                changed = false;
            }

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if event == Event::Key(KeyCode::Esc.into()) {
                                if self.code.abs_path != initial_abs_path {
                                    self.open_file(&initial_abs_path).await;
                                }
                                self.r = r; self.c = c;
                                self.handle_movement();
                                self.selection.clean();

                                self.upd = true;
                                self.tree_view.upd = true;
                                return;
                            }
                            if event == Event::Key(KeyCode::Down.into()) && selected < search_results.len() - 1 {
                                changed = true;
                                selected += 1;
                                self.upd = true;
                                // self.tree_view.upd = true;
                            }

                            if event == Event::Key(KeyCode::Up.into()) && selected > 0 {
                                changed = true;
                                selected -= 1;
                                self.upd = true;
                                // self.tree_view.upd = true;
                            }

                            if let Event::Resize(w, h) = event {
                                self.upd = true;
                                self.tree_view.upd = true;
                                self.resize(w as usize, h as usize);
                                changed = true;
                            }

                            if event == Event::Key(KeyCode::Enter.into())
                            || event == Event::Key(KeyCode::Tab.into()) {
                                self.upd = true;
                                self.tree_view.upd = true;
                                return;
                            }
                        }
                        Some(Err(e)) => { debug!("Error: {:?}\r", e); return; },
                        None => break,
                    }
                }
            };
        }
    }

    pub fn draw_search_result(&mut self,
        height: usize, width:usize, fromy: usize,
        options: &Vec<(String, search::search::SearchResult)>,
        selected: usize, offset: usize, elapsed: u128
    ) {
        let limit = self.width - self.lp_width - self.ln_width - self.lns_width - 1;

        let options: Vec<String> = options.iter().enumerate().map(|(i, (path, sr))| {
            let prefix = format!("{}/{} {}:{} ", i+1, options.len(), sr.line,  sr.column);
            let path = path.chars().take(limit-prefix.len()).collect::<String>();
            format!("{} {}", prefix, path)
        }).collect();

        let width = options.iter().map(|o| o.len()).max().unwrap_or(width);

        for row in 0..options.len() {
            if row >= options.len() || row >= height { break; }
            let option = &options[row + offset];

            let is_selected = selected == row + offset;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let label = format!(" {:width$} ", option, width = width);

            queue!(stdout(),
                cursor::MoveTo((self.lp_width + self.ln_width + self.lns_width - 1) as u16, (row + fromy) as u16),
                BColor(bgcolor), FColor(self.lncolor), Print(label),
                terminal::Clear(ClearType::UntilNewLine), BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        let status = format!(
            "global search on '{}', elapsed {} ms",
            &self.search.pattern,  elapsed
        );

        queue!(stdout(),
            cursor::MoveTo((self.lp_width + 1) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(status),
        );


        stdout().flush().expect("cant flush");


        // draw inside left panel
        // if self.lp_width == 0 { return; }
        // let mut stdout = stdout();
        // let limit = self.lp_width;

        // let options: Vec<String> = options.iter().enumerate().map(|(i, (path, sr))| {
        //     let prefix = format!("{}/{} {}:{} ", i+1, options.len(), sr.line,  sr.column);
        //     let path = path.chars().take(limit-prefix.len()).collect::<String>();
        //     format!("{} {}", prefix, path)
        // }).collect();

        // let mut count = 0;
        // let mut row = 0;
        // loop {
        //     if row >= options.len() || row >= height { break; }
        //     let option = &options[row + offset];

        //     let isRowSelected = selected == row + offset;
        //     let bgcolor = if isRowSelected { Color::Grey } else { Color::Reset };

        //     let label = format!(" {:width$}", option, width = self.lp_width-3);

        //     queue!(stdout,
        //         cursor::MoveTo(1, row  as u16),
        //         BColor(bgcolor), FColor(self.lncolor), Print(label),
        //         BColor(Color::Reset), FColor(Color::Reset),
        //     );

        //     queue!(stdout, FColor(Color::DarkGrey), Print('│'));
        //     count += 1;
        //     row += 1;
        // }

        // while count < self.height { // fill empty space
        //     queue!(stdout, cursor::MoveTo(0, count as u16));
        //     queue!(stdout, Print(" ".repeat(self.lp_width-1)));
        //     queue!(stdout, FColor(Color::DarkGrey), Print('│'));
        //     count += 1;
        // }
        // stdout.flush().expect("cant flush");
    }

    async fn undo_cursor(&mut self) {
        match self.cursor_history.pop() {
            Some(cursor_position) => {
                self.cursor_history_undo.push(CursorPosition{
                    filename: self.code.abs_path.clone(),
                    row: self.r, col: self.c,
                    y: self.y, x: self.x,
                });

                if cursor_position.filename != self.code.abs_path {
                    self.open_file(&cursor_position.filename).await;
                }
                self.r = cursor_position.row;
                self.c = cursor_position.col;
                self.y = cursor_position.y;
                self.x = cursor_position.x;
                self.upd = true;
                self.handle_movement();
            },
            None => {},
        }
    }
    async fn redo_cursor(&mut self) {
        match self.cursor_history_undo.pop() {
            Some(cursor_position) => {
                self.cursor_history.push(CursorPosition{
                    filename: self.code.abs_path.clone(),
                    row: self.r, col: self.c,
                    y: self.y, x: self.x,
                });

                if cursor_position.filename != self.code.abs_path {
                    self.open_file(&cursor_position.filename).await;
                }
                self.r = cursor_position.row;
                self.c = cursor_position.col;
                self.y = cursor_position.y;
                self.x = cursor_position.x;
                self.upd = true;
                self.handle_movement();
            },
            None => {},
        }
    }

    async fn move_line_down(&mut self) {
        if self.r >= self.code.len_lines()-1 { return }

        let line1len = self.code.line_len(self.r);
        let line2len = self.code.line_len(self.r+1);
        let line1 = self.code.get_text(self.r,0, self.r, line1len);
        let line2 = self.code.get_text(self.r+1,0, self.r+1, line2len);

        let success = self.code.move_line_down(self.r);
        if !success { return }

        if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
            let mut lsp = lsp.lock().await;
            lsp.did_change(self.r, 0, self.r, line1len, &self.code.abs_path, "").await;
            lsp.did_change(self.r, 0, self.r, 0, &self.code.abs_path, &line2).await;
            lsp.did_change(self.r+1, 0, self.r+1, line2len, &self.code.abs_path, "").await;
            lsp.did_change(self.r+1, 0, self.r+1, 0, &self.code.abs_path, &line1).await;
        }

        self.r += 1;

        self.selection.clean();
        self.selection.keep_once = false;
        self.upd = true;
        self.clean_diagnostics();
    }

    async fn move_line_up(&mut self)  {
        if self.r == 0 { return }
        if self.r >= self.code.len_lines() { return }

        self.r -= 1;
        self.move_line_down().await;
        self.r -= 1;
    }

    fn select_more(&mut self) {
        let next = match self.node_path.as_mut() {
            Some(node_path) => {
                if node_path.row == self.r && node_path.column == self.c {
                    true
                } else {
                    self.node_path = self.code.get_node_path(self.r, self.c);
                    false
                }
            },
            None => {
                self.node_path = self.code.get_node_path(self.r, self.c);
                false
            },
        };


        let node = if next && self.node_path.is_some() {
            self.node_path.as_mut().unwrap().next_node()
        } else {
            self.node_path.as_mut().unwrap().current_node()
        };

        if let Some(next_node) = node {
            self.selection.set_start(next_node.0.row, next_node.0.column);
            self.selection.set_end(next_node.1.row, next_node.1.column);
            self.selection.active = true;
            self.upd = true;
        }
    }

    fn select_less(&mut self) {
        let prev = match self.node_path.as_mut() {
            Some(node_path) => {
                if node_path.row == self.r && node_path.column == self.c {
                    true
                } else {
                    self.node_path = self.code.get_node_path(self.r, self.c);
                    false
                }
            },
            None => {
                self.node_path = self.code.get_node_path(self.r, self.c);
                false
            },
        };


        let node = if prev && self.node_path.is_some() {
            self.node_path.as_mut().unwrap().prev_node()
        } else {
            self.node_path.as_mut().unwrap().current_node()
        };


        if let Some(next_node) = node {
            self.selection.set_start(next_node.0.row, next_node.0.column);
            self.selection.set_end(next_node.1.row, next_node.1.column);
            self.selection.active = true;
            self.upd = true;
        } else {
            self.selection.clean();
            self.upd = true;
        }
    }

}

impl Drop for Editor {
    fn drop(&mut self) {
        Self::deinit()
    }
}
