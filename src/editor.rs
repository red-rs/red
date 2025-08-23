use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{stdout, Write};
use std::path::Path;
use std::time::Instant;
use std::time;
use anyhow::anyhow;
use log2::{debug};
use unicode_width::{UnicodeWidthStr, UnicodeWidthChar};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event,
    EventStream, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind, 
    EnableBracketedPaste, DisableBracketedPaste,
};
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, ClearType,
    EnterAlternateScreen, LeaveAlternateScreen
};
use crossterm::{
    cursor, execute, queue,
    style::{Color, SetBackgroundColor as BColor, SetForegroundColor as FColor},
    terminal,
};
use crossterm::cursor::{SetCursorStyle};
use futures::{future::FutureExt, select, StreamExt};
use crate::code::{Code, NodePath};
use crate::config::Config;
use crate::search::{Search, SearchResult};
use crate::lsp::{self, Lsp};
use crate::process::Process;
use crate::selection::Selection;
use crate::utils::{CursorHistory, CursorPosition, score_matches, ClickType};
use crate::{search::{search_in_directory}, utils};
use crate::tree;

use tokio::sync::Mutex;
use std::sync::Arc;
use notify::{recommended_watcher, RecursiveMode, Watcher, event::ModifyKind};
use crate::utils::Rect;
use std::cell::RefCell;
use tokio::sync::mpsc;

// keyword and ratatui style
type Theme = HashMap<String, String>;
// start byte, end byte, style
type Hightlight = (usize, usize, Color);
// start offset, end offset
type HightlightCache = HashMap<(usize, usize), Vec<Hightlight>>;


/// Represents a text editor.
pub struct Editor {
    /// Config from config.toml.
    config: Config,
    /// Text buffer to display.
    code: Code,
    /// Local clipboard
    clipboard: Option<String>,
    /// Terminal height.
    height: usize,
    /// Terminal width.
    width: usize,
    /// Cursor row.
    r: usize,
    /// Cursor column.
    c: usize,
    /// Scroll row offset.
    x: usize,
    /// Scroll column offset.
    y: usize,
    /// Left panel width
    lp_width: usize,
    /// Update screen flag.
    upd: bool,
    /// Theme for syntax highlighting and etc
    theme: HashMap<String, String>,
    /// Cache forghighlights intervals
    highlights_cache: RefCell<HightlightCache>,
    /// Color for line number.
    lncolor: Color,
    /// Color for status line.
    scolor: Color,
    /// Color for selection.
    selcolor: Color,
    /// Color for errors.
    ecolor: Color,
    /// Color for line buttons.
    lbcolor: Color,

    /// Mouse selection range.
    selection: Selection,

    /// process
    process: Process,

    /// lsp servers for a language
    lang2lsp: HashMap<String,Arc<Mutex<Lsp>>>,
    lsp_status: Arc<Mutex<String>>,

    /// diagnostics or errors to inline display
    diagnostics: Arc<Mutex<HashMap<String, lsp_types::PublishDiagnosticsParams>>>,
    diagnostics_sender: Option<tokio::sync::mpsc::Sender<lsp_types::PublishDiagnosticsParams>>,

    /// tree view
    tree_view: tree::TreeView,

    /// opened text buffers
    codes: HashMap<String, Code>,

    /// search
    search: Search,

    overlay_lines: HashSet<usize>,

    /// cursor history
    cursor_history: CursorHistory,

    is_lp_focused: bool,

    node_path: Option<NodePath>,

    watcher: Option<notify::RecommendedWatcher>,
    self_update: bool,

    last_click: Option<(Instant, usize)>,
    last_last_click: Option<(Instant, usize)>,

    hovered_runnable_line: Option<usize>,
}

impl Editor {
    pub fn new(config: Config) -> Self {
        Editor {
            config,
            code: Code::new(),
            clipboard: None,
            height: 0,
            width: 0,
            r: 0, c: 0, x: 0, y: 0,
            lncolor: Color::Reset,
            scolor: Color::Reset,
            selcolor: Color::Reset,
            ecolor: Color::Reset,
            lbcolor: Color::Reset,
            upd: true,
            theme: HashMap::new(),
            highlights_cache: RefCell::new(HashMap::new()),
            selection: Selection::new(),
            process: Process::new(),
            lang2lsp: HashMap::new(),
            lsp_status: Arc::new(Mutex::new(String::new())),
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
            diagnostics_sender: None,
            tree_view: tree::TreeView::new(".".to_string()),
            lp_width: 0,
            codes: HashMap::new(),
            search: Search::new(),
            overlay_lines: HashSet::new(),
            cursor_history: CursorHistory::new(),
            is_lp_focused: false,
            node_path: None,
            watcher: None,
            self_update: false,
            last_click: None,
            last_last_click: None,
            hovered_runnable_line: None,
        }
    }

    pub fn load_file(&mut self, fname: &str) {
        let buf = Code::from_file(&fname, &self.config);

        match buf {
            Ok(code) => {
                self.code = code;
                self.r = 0; self.c = 0; self.y = 0; self.x = 0;
                self.selection.clean();
                self.reset_highlight_cache();
            }
            Err(_) => {},
        }
    }

    pub fn open_left_panel(&mut self) {
        self.lp_width = self.config.left_panel_width.unwrap_or(25);
        self.is_lp_focused = true;
        self.resize(self.width, self.height);
    }

    pub fn close_left_panel(&mut self) {
        self.lp_width = 0;
        self.resize(self.width, self.height);
    }

    pub fn left_panel_toggle(&mut self) {
        if self.lp_width > 0 { self.lp_width = 0; }
        else { self.lp_width = self.config.left_panel_width.unwrap_or(25); }

        self.resize(self.width, self.height);
        self.tree_view.set_width(self.lp_width);
        self.upd = true;
    }

    pub fn get_line_number_width(&self) -> usize {
        let total_lines = self.code.len_lines();
        let max_line_number = total_lines.max(1);
        let line_number_digits = max_line_number.to_string().len().max(5);
        line_number_digits + 2
    }

    pub fn init(&mut self) -> anyhow::Result<()> {
        execute!(stdout(), EnterAlternateScreen)?;
        execute!(stdout(), EnableMouseCapture)?;
        enable_raw_mode()?;
        execute!(stdout(), cursor::Hide)?;
        execute!(stdout(), SetCursorStyle::DefaultUserShape)?;
        execute!(stdout(), EnableBracketedPaste)?;
        stdout().flush()?;

        let (w, h) = terminal::size()?;
        self.resize(w as usize, h as usize);
        self.tree_view.set_width(self.lp_width);
        self.configure_theme();
        Ok(())
    }

    pub fn deinit() -> anyhow::Result<()> {
        disable_raw_mode()?;
        execute!(stdout(), LeaveAlternateScreen)?;
        execute!(stdout(), DisableMouseCapture)?;
        execute!(stdout(), DisableBracketedPaste)?;
        queue!(stdout(), cursor::Show)?;
        Ok(())
    }

    pub fn handle_panic(&self) {
        let default_panic = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Self::deinit().expect("Error deinit");
            default_panic(info);
            std::process::exit(1);
        }));

        ctrlc::set_handler(move || {
            Self::deinit().expect("Error deinit");
        })
        .expect("Error setting Ctrl-C handler");
    }

    fn configure_theme(&mut self) {
        let theme_path = &self.config.theme;
        let theme_content = crate::config::get_file_content(theme_path).unwrap();
        let theme_yaml = serde_yaml::from_str(&theme_content)
            .expect("Failed to parse theme yaml file");
        self.theme = utils::yaml_to_map(theme_yaml);

        self.lncolor = self.theme.get("lncolor").map(|c| utils::hex_to_color(c))
            .unwrap_or(Color::AnsiValue(247));
        self.scolor = self.theme.get("scolor").map(|c| utils::hex_to_color(c))
            .unwrap_or(Color::AnsiValue(247));
        self.selcolor = self.theme.get("selcolor").map(|c| utils::hex_to_color(c))
            .unwrap_or(Color::AnsiValue(247));
        self.ecolor = self.theme.get("ecolor").map(|c| utils::hex_to_color(c))
            .unwrap_or(Color::AnsiValue(247));
        self.lbcolor = self.theme.get("lbcolor").map(|c| utils::hex_to_color(c))
            .unwrap_or(Color::AnsiValue(87));

        let dircolor = self.theme.get("dircolor").map(|c| utils::hex_to_color(c));
        self.tree_view.set_dir_color(dircolor.unwrap_or(Color::Reset));

        let filecolor = self.theme.get("filecolor").map(|c| utils::hex_to_color(c));
        self.tree_view.set_file_color(filecolor.unwrap_or(Color::Reset));

        let activefilecolor = self.theme.get("activefilecolor").map(|c| utils::hex_to_color(c));
        self.tree_view.set_active_file_color(activefilecolor.unwrap_or(Color::Reset));
    }

    pub async fn start(&mut self) {
        let init_result = self.init();
        if let Err(e) = init_result {
            eprintln!("Cannot init screen {:?}", e);
            return;
        }

        self.draw().await;

        let (diagnostic_send, mut diagnostic_rx) = mpsc::channel::<lsp_types::PublishDiagnosticsParams>(1);
        self.diagnostics_sender = Some(diagnostic_send.clone());

        self.init_new_lsp();

        let (watch_tx, mut watch_rx) = mpsc::channel::<notify::Result<notify::Event>>(32);

        let mut watcher = recommended_watcher(move |res| {
            let _ = watch_tx.blocking_send(res);
        }).unwrap();

        let p = Path::new(&self.code.abs_path);
        let _ = watcher.watch(p, RecursiveMode::Recursive);

        self.watcher = Some(watcher);

        let mut reader = EventStream::new();

        loop {
            let event = reader.next().fuse();

            tokio::select! {
                Some(event) = watch_rx.recv() => {
                    self.handle_watch_event(event).await;
                }

                Some(upd) = diagnostic_rx.recv() => {
                    self.handle_diagnostic_update(upd).await;
                }

                Some(Ok(event)) = event => {
                    if self.is_quit_event(&event) { break }
                    self.handle_terminal_event(event).await;
                }
            };
        }
    }

    async fn handle_watch_event(
        &mut self, res: Result<notify::Event, notify::Error>
    ) {
        match res {
            Ok(event) => {
                // println!("File event: {:?}", event);
                match event.kind {

                    notify::EventKind::Modify(ModifyKind::Data(_)) => {
                        let selfpath = Path::new(&self.code.abs_path);

                        if self.self_update {
                            self.self_update = false;
                            return;
                        }

                        let is_need_update = event.paths.iter().any(|p| p == &selfpath);

                        if is_need_update {
                            debug!("Self update detected");
                            let _ = self.code.reload();
                            self.lsp_update().await;
                            self.upd = true;
                            self.draw().await;
                        }
                    }
                    _ => {}
                }
            }
            Err(_) => {}
        }
    }

    fn is_quit_event(&mut self, event: &Event) -> bool {
        match event {
            Event::Key(e) => {
                if e.modifiers == KeyModifiers::CONTROL &&
                   e.code == KeyCode::Char('q') {
                    return true;
                }
            }
            _ => {}
        }
        return false;
    }

    async fn handle_terminal_event(&mut self, event: Event) {
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
                #[cfg(target_os = "windows")] { // skip press event on windows
                    if e.kind == KeyEventKind::Press {
                        anyhow::bail!("skip press event on windows")
                    }
                };
                self.handle_keyboard(e).await;
                self.draw().await;
            }
            Event::FocusGained => {}
            Event::FocusLost => {}
            Event::Paste(s) => {
                self.paste(s).await;
                self.draw().await;
            }
        }
    }

    async fn handle_diagnostic_update(&mut self, upd: lsp_types::PublishDiagnosticsParams) {
        let filename = upd.uri.clone().to_string();
        self.diagnostics.lock().await.insert(filename, upd);
        self.upd = true;
        self.draw().await;
    }

    fn resize(&mut self, w: usize, h: usize) {
        if w == 0 || h == 0 { return; }
        if w != self.width {
            self.width = w;
        }
        if h != self.height {
            self.height = h;
        }

        self.upd = true;
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
                match event.code {
                    KeyCode::Up => self.select_more(),
                    KeyCode::Down => self.select_less(),
                    KeyCode::Left =>  self.handle_left_word(),
                    KeyCode::Right => self.handle_right_word(),
                    KeyCode::Backspace => self.handle_cut_line().await,
                    _ => debug!("event.code {:?}", event.code),
                }
                return;
            }

            KeyModifiers::CONTROL => {
                match event.code {
                    KeyCode::Char('s') => self.save(),
                    KeyCode::Char('c') => self.copy_to_clipboard(None),
                    KeyCode::Char('v') => self.paste_from_clipboard().await,
                    KeyCode::Char('d') => self.handle_duplicate().await,
                    KeyCode::Char('f') => self.handle_local_search().await,
                    KeyCode::Char('r') => self.references().await,
                    KeyCode::Char('g') => self.definition().await,
                    KeyCode::Char('z') => self.undo().await,
                    KeyCode::Char('y') => self.redo().await,
                    KeyCode::Char('o') => self.undo_cursor().await,
                    KeyCode::Char('p') => self.redo_cursor().await,
                    KeyCode::Char('e') => self.handle_errors().await,
                    KeyCode::Char('h') => self.hover().await,
                    KeyCode::Char('t') => self.toggle_left_panel(),
                    KeyCode::Char(' ') => self.completion().await,
                    KeyCode::Char('x') => {
                        self.copy_to_clipboard(None);
                        self.handle_cut().await;
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
            KeyCode::PageUp => self.handle_page_up(),
            KeyCode::PageDown => self.handle_page_down(),
            _ => {}
        }


        if self.selection.active || self.selection.keep_once  {
            self.selection.clean();
            self.selection.keep_once = false;
            self.upd = true;
        }
    }

    fn toggle_left_panel(&mut self) {
        if self.lp_width == 0 {
            self.is_lp_focused = true;
            self.left_panel_toggle();
        }
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
            KeyModifiers::NONE => {},
            KeyModifiers::SHIFT => {},
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
                            self.save_cursor_to_history();

                            if self.tree_view.is_search() {
                                self.tree_view.clear_search();
                                self.tree_view.expand_root();
                                self.tree_view.set_scroll(0);
                                self.tree_view.find_expand_by_fullpath(&path);
                            }

                            self.open_file(&path).await;
                            self.is_lp_focused = false;
                        }
                        else {
                            let _ = node.toggle();
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

    pub async fn open_file(&mut self, path: &String) {
        let is_open = self.codes.contains_key(path);

        if !is_open {
            self.code.set_cursor_position(self.r, self.c, self.y, self.x);

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
            if let Some(watcher) = self.watcher.as_mut() {
                let dir = Path::new(path);
                let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
            }
        } else {
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

    fn is_on_divider(&self, e: MouseEvent) -> bool {
        let col = e.column as usize;
        let line = e.row as usize + self.y;

        let around_line_number = col >= self.lp_width.saturating_sub(1)
            && col < self.lp_width + 3
            && !self.code.is_runnable(line);

        // let on_left_panel_edge = col > 0 && col == self.lp_width.saturating_sub(1);

        around_line_number
    }

    fn is_on_runnable_button(&self, col: u16) -> bool {
        col as usize == self.lp_width
    }

    async fn handle_mouse(&mut self, e: MouseEvent) {
        self.is_lp_focused = (e.column as usize) < self.lp_width;

        match e.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.is_on_divider(e) {
                    self.tree_view.set_moving(true);
                    self.upd = true;
                    return;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.tree_view.is_moving() {
                    let editor_min_width = 20;
                    let max = self.width.saturating_sub(editor_min_width);
                    let width = e.column.clamp(1, max as u16) as usize;
                    self.lp_width = width;
                    self.tree_view.set_width(width);
                    self.upd = true;
                    return;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.tree_view.is_moving() {
                    self.tree_view.set_moving(false);
                    self.upd = true;
                    return;
                }
            }
            _ => {}
        }

        let line = e.row as usize + self.y;

        // check runnable button first
        if self.is_on_runnable_button(e.column) && self.code.is_runnable(line) {
            self.hovered_runnable_line = Some(line);
            self.draw_run_button(e.row as usize, Color::DarkBlue);
            self.draw_cursor();
            if e.kind == MouseEventKind::Up(MouseButton::Left) {
                if let Some(runnable) = self.code.get_runnable(line) {
                    let _ = self.process.run_tmux(&runnable.cmd).await;
                }
            }
            return;
        }

        // if button hovered, draw it back
        if let Some(prev_line) = self.hovered_runnable_line.take() {
            if self.code.is_runnable(prev_line) {
                let y = prev_line - self.y;
                self.draw_run_button(y as usize, self.lbcolor);
            }
            self.draw_cursor();
        }

        if self.is_lp_focused {
            self.handle_mouse_tree(e).await;
        } else {
            let area = Rect::new(
                self.lp_width as u16, 0,
                self.width as u16, self.height as u16,
            );
            self.handle_mouse_editor(e, &area).await;
        }
    }

    async fn handle_mouse_tree(&mut self, e: MouseEvent) {
        match e.kind {
            MouseEventKind::ScrollUp => {
                self.tree_view.scroll_up();
                self.upd = true;
            }
            MouseEventKind::ScrollDown => {
                self.tree_view.scroll_down();
                self.upd = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if e.column as usize == self.lp_width.saturating_sub(1) {
                    self.tree_view.set_moving(true);
                    self.upd = true;
                    return;
                }

                self.tree_view.set_moving(false);

                let maybe_clicked_node = self.tree_view.find_with_depth(e.row as usize);

                if let Some((clicked_node, depth)) = maybe_clicked_node {
                    let name = clicked_node.name();
                    let name_width = unicode_width::UnicodeWidthStr::width(name.as_str());
                    let name_start = 1 + depth as u16;
                    let end = name_start + name_width as u16;

                    let name_clicked = e.column >= name_start && e.column < end;
                    if !name_clicked { return; }

                    if clicked_node.is_file() {
                        let path = clicked_node.fullpath();
                        self.save_cursor_to_history();
                        self.tree_view.set_active(path.clone());
                        self.open_file(&path).await;
                        self.save_cursor_to_history();
                    } else {
                        let _ = clicked_node.toggle();
                    }

                    self.tree_view.set_selected(e.row as usize);
                    self.upd = true;
                }
            }
            _ => {}
        }
    }

    async fn handle_mouse_editor(&mut self, e: MouseEvent, area: &Rect) {

        // handle clicks with modifier keys first
        match (e.modifiers, e.kind) {
            (KeyModifiers::CONTROL, MouseEventKind::Down(MouseButton::Left)) => {
                if let Some(cursor) = self.cursor_from_mouse(e.column, e.row, area) {
                    (self.r, self.c) = self.code.point(cursor);
                    // self.handle_mouse_click(self.r, self.c);
                    self.definition().await;
                    return;
                }
            }
            (KeyModifiers::ALT, MouseEventKind::Down(MouseButton::Left)) => {
                if let Some(cursor) = self.cursor_from_mouse(e.column, e.row, area) {
                    (self.r, self.c) = self.code.point(cursor);
                    // self.handle_mouse_click(self.r, self.c);
                    self.references().await;
                    return;
                }
            }
            _ => {}
        }

        match e.kind {
            MouseEventKind::ScrollUp => self.scroll_up(),
            MouseEventKind::ScrollDown => self.scroll_down(),
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = self.cursor_from_mouse(e.column, e.row, area);

                if let Some(cursor) = pos {
                    let now = Instant::now();
                    let click_type = ClickType::from_click_history(
                        now, cursor, self.last_click, self.last_last_click,
                    );
                    let (start, end) = match click_type {
                        ClickType::Triple => self.code.line_boundaries(cursor),
                        ClickType::Double => self.code.word_boundaries(cursor),
                        ClickType::Single => (cursor, cursor),
                    };
                    self.last_last_click = self.last_click;
                    self.last_click = Some((now, cursor));

                    let start_point = self.code.point(start);
                    let end_point = self.code.point(end);
                    self.selection.set_start(start_point.0, start_point.1);
                    self.selection.set_end(end_point.0, end_point.1);
                    self.selection.active = true;
                    self.upd = true;
                    self.r = end_point.0;
                    self.c = end_point.1;
                    self.save_cursor_to_history();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let pos = self.cursor_from_mouse(e.column, e.row, area);
                if let Some(cursor) = pos {
                    let point = self.code.point(cursor);
                    self.selection.set_end(point.0, point.1);
                    self.r = point.0;
                    self.c = point.1;
                    self.selection.active = true;
                    self.upd = true;
                }
            }
            _ => {}
        }
    }

    fn cursor_from_mouse(
        &self, mouse_x: u16, mouse_y: u16, area: &Rect
    ) -> Option<usize> {

        let line_number_width = self.get_line_number_width() as u16;

        if mouse_y < area.top()
            || mouse_y >= area.bottom()
            || mouse_x < area.left() + line_number_width
        {
            return None;
        }

        let clicked_row = (mouse_y - area.top()) as usize + self.y;
        if clicked_row >= self.code.len_lines() {
            return None;
        }

        let clicked_col = (mouse_x - area.left() - line_number_width) as usize;

        let line_start_char = self.code.line_to_char(clicked_row);
        let line_len = self.code.line_len(clicked_row);

        let start_col = self.x.min(line_len);
        let end_col = line_len;

        let char_start = line_start_char + start_col;
        let char_end = line_start_char + end_col;

        let visible_chars = self.code.char_slice(char_start, char_end);

        let mut current_col = 0;
        let mut char_idx = start_col;

        for ch in visible_chars.chars() {
            let ch_width = ch.width().unwrap_or(1);
            if current_col + ch_width > clicked_col {
                break;
            }
            current_col += ch_width;
            char_idx += 1;
        }

        let line = self.code.char_slice(line_start_char, line_start_char + line_len);

        let visual_width: usize = line.to_string().width();

        if clicked_col + self.x >= visual_width {
            let mut end_idx = line.len_chars();
            if end_idx > 0 && line.char(end_idx - 1) == '\n' {
                end_idx -= 1;
            }
            char_idx = end_idx;
        }

        Some(line_start_char + char_idx)
    }

    fn status_line(&self) -> String {
        // let buttons = format!("  {} {} {} {} {}", '☰','☌', '', '▶', '⛭' );
        let buttons = "".to_string();

        if self.code.file_name.is_empty() {
            format!("  {}", buttons)
        } else {
            let changed = if self.code.changed { "*" } else { " " };
            format!("  {}:{} {} {}{}{}",
                self.r + 1, self.c + 1, self.code.lang, self.code.file_name, changed,
                buttons
            )
        }
    }

    pub fn clear_all(&self) {
        let mut stdout = stdout();
        let _ = queue!(stdout, terminal::Clear(ClearType::All));
        stdout.flush().expect("flush");
    }

    fn cached_highlight_interval(
        &self, start: usize, end: usize, theme: &Theme
    ) -> Vec<Hightlight> {
        let mut cache = self.highlights_cache.borrow_mut();
        let key = (start, end);
        if let Some(v) = cache.get(&key) {
            return v.clone();
        }

        let highlights = self.code.highlight_interval(start, end, theme);
        cache.insert(key, highlights.clone());
        highlights
    }

    fn reset_highlight_cache(&self) {
        self.highlights_cache.borrow_mut().clear();
    }

    async fn draw(&mut self) {
        // let start = time::Instant::now();

        if self.height < 1 { return }

        let is_file_empty = self.code.file_name.is_empty();

        if is_file_empty {
            let _ = queue!(stdout(), cursor::Hide);
            if self.tree_view.is_search(){
                let _ = queue!(stdout(), cursor::Show);
            }
            self.tree_view.draw(is_file_empty);
            self.draw_logo();
            self.draw_status();
            self.tree_view.draw_search();
            stdout().flush().expect("flush");
            return;
        }

        self.tree_view.draw(is_file_empty);
        self.draw_cursor();
        self.tree_view.draw_search();

        if !self.upd { return }

        self.draw_editor();
        self.draw_status();
        // self.draw_ttr(start);
        self.draw_cursor();

        self.tree_view.draw_search();

        stdout().flush().expect("flush");
        self.upd = false;
    }

    #[allow(dead_code)]
    fn draw_ttr(&mut self, start: time::Instant) {
        let elapsed = time::Instant::now() - start;
        let ttr = format!(" {:?} ms  {:?} ns",
            elapsed.as_millis(), elapsed.as_nanos()
        );

        let _ = queue!(
            stdout(),
            cursor::MoveTo((self.width - ttr.len() -1) as u16, (self.height) as u16),
            FColor(self.lncolor),
            Print(ttr),
        );

        self.draw_cursor();
    }

    fn draw_editor(&self) {
        let mut stdout = stdout();
        let _ = queue!(stdout, cursor::Hide);

        let area = Rect::new(
            (self.lp_width) as u16, 0 as u16,
            self.width as u16, self.height as u16,
        );

        let total_lines = self.code.len_lines();
        let line_number_width = self.get_line_number_width();

        let _ = queue!(stdout, cursor::MoveTo(area.left(), area.top()));

        let line2error = self.get_lines_errors(self.y, self.y + self.height);
        let mut last_line_drawn = 0;

        // draw line numbers and text
        for screen_y in 0..(area.height as usize) {
            if self.overlay_lines.contains(&screen_y) { continue }

            let line_idx = self.y + screen_y;
            last_line_drawn = screen_y;
            if line_idx >= total_lines { break }

            let draw_y = area.top() + screen_y as u16;
            if draw_y >= area.bottom() { break }

            let _ = queue!(stdout, cursor::MoveTo(area.left(), area.top() + draw_y));

            if self.code.is_runnable(line_idx) {
                self.draw_run_button(screen_y, self.lbcolor);
            } else {
                let _ = queue!(stdout, BColor(Color::Reset), FColor(Color::Reset), Print(" "));
            }

            let line_number = format!("{:^width$}", line_idx + 1, width = line_number_width-1);
            let _ = queue!(stdout, BColor(Color::Reset), FColor(self.lncolor), Print(line_number));

            let line_len = self.code.line_len(line_idx);

            let available_width = (area.width as usize)
                .saturating_sub(line_number_width)
                .saturating_sub(area.left() as usize);

            let start_col = self.x.min(line_len);
            
            // Calculate how many characters can fit in the available width
            let mut max_chars = 0;
            let mut current_width = 0;
            let line_start_char = self.code.line_to_char(line_idx);
            
            // First pass: count how many characters fit
            let line_chars = self.code.char_slice(line_start_char, line_start_char + line_len);
            for ch in line_chars.chars().skip(start_col) {
                let ch_width = ch.width().unwrap_or(1);
                if current_width + ch_width > available_width {
                    break;
                }
                current_width += ch_width;
                max_chars += 1;
            }
            
            let end_col = start_col + max_chars;
            let char_start = line_start_char + start_col;
            let char_end = line_start_char + end_col;

            let visible_chars = self.code.char_slice(char_start, char_end);
            let displayed_line: String = visible_chars.chars()
                .map(|c| if c == '\t' { ' ' } else { c })
                .filter(|c| !c.is_control())  // remove CR, LF and other control characters
                .collect();

            let start_byte = self.code.char_to_byte(char_start);
            let end_byte = self.code.char_to_byte(char_end);

            let highlights = self.cached_highlight_interval(start_byte, end_byte, &self.theme);

            let mut vis_x = 0; 
            let mut char_pos = start_col; 
            let mut byte_idx_in_rope = start_byte;

            for ch in displayed_line.chars() {
                let ch_width = ch.width().unwrap_or(1);
                
                if vis_x + ch_width > available_width { break }

                let mut fcolor = Color::Reset;
                for &(start, end, s) in &highlights {
                    if start <= byte_idx_in_rope && byte_idx_in_rope < end {
                        fcolor = s;
                        break;
                    }
                }

                let bcolor = match self.selection.is_selected(line_idx, char_pos) {
                    true => self.selcolor,
                    false => Color::Reset,
                };

                let _ = queue!(stdout, FColor(fcolor), BColor(bcolor), Print(ch));
                vis_x += ch_width;
                char_pos += 1; 
                byte_idx_in_rope += ch.len_utf8();
            }

            if let Some(errors) = line2error.get(&line_idx) {
                let x_error = area.left() as usize + line_number_width + end_col;
                self.draw_error(errors, x_error, screen_y);
            }

            let _ = queue!(stdout, BColor(Color::Reset), terminal::Clear(ClearType::UntilNewLine));
            // stdout.flush().expect("flush");
        }

        if last_line_drawn + 1 < self.height {
            // fill empty space
            for row in last_line_drawn..self.height {
                if self.overlay_lines.contains(&row) { continue }
                let _ = queue!(stdout, cursor::MoveTo(area.left(), row as u16));
                let _ = queue!(stdout, BColor(Color::Reset), terminal::Clear(ClearType::UntilNewLine));
                // stdout.flush().expect("flush");
            }
        }

    }

    fn get_lines_errors(
        &self,
        start_row: usize,
        end_row: usize,
    ) -> HashMap<usize, Vec<lsp_types::Diagnostic>> {
        let uri = format!("file://{}", self.code.abs_path);
        let diagnostics = self.diagnostics.clone();
        let maybe_diagnostics = diagnostics.try_lock().unwrap();
        let maybe_diagnostics = maybe_diagnostics.get(&uri);

        match maybe_diagnostics {
            Some(d) => {
                let mut errors = HashMap::new();

                for diag in d.diagnostics.iter() {
                    let line = diag.range.start.line as usize;
                    if start_row <= line && line <= end_row {
                        errors
                            .entry(line)
                            .or_insert_with(Vec::new)
                            .push(diag.clone());
                    }
                }

                errors
            }
            None => HashMap::new(),
        }
    }

    fn draw_error(&self, error_messages: &[lsp_types::Diagnostic], x: usize, y: usize) {
        let space = 5;
        let prefix = " ".repeat(space);

        for (i, msg) in error_messages.iter().enumerate() {
            let draw_y = y + i;
            if x >= self.width || draw_y >= self.height {
                break;
            }

            let available_width = self.width.saturating_sub(x);
            let message_limit = available_width.saturating_sub(space);

            let m: String = msg.message
                .replace('\n', " ")
                .chars()
                .take(message_limit)
                .collect();

            let full_msg = format!("{prefix}{m}");

            let color = match msg.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) => Color::Red,
                Some(lsp_types::DiagnosticSeverity::WARNING) => Color::Yellow,
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => Color::Cyan,
                Some(lsp_types::DiagnosticSeverity::HINT) => Color::Green,
                _ => Color::Reset,
            };

            let _ = queue!(
                stdout(),
                cursor::MoveTo(x as u16, draw_y as u16),
                BColor(Color::Reset),
                FColor(color),
                Print(full_msg),
                FColor(Color::Reset)
            );
        }
    }

    fn draw_cursor(&mut self) {
        if self.code.file_name.is_empty() { return; }

        let line_number_digits = self.get_line_number_width();
        let vertical_fit = (self.r >= self.y) && (self.r - self.y) < self.height;
        // Calculate visual cursor position for horizontal fit check
        let line_start_char = self.code.line_to_char(self.r);
        let line_text = self.code.char_slice(line_start_char, line_start_char + self.c);
        let visual_cursor_pos = line_text.to_string().width();
        
        let horizontal_fit = (self.c >= self.x)
            && (self.lp_width + line_number_digits + visual_cursor_pos - self.x) < self.width;

        if !vertical_fit || !horizontal_fit {
            return;
        }

        let out_left = self.c < self.x;
        let out_right = self.lp_width + line_number_digits + visual_cursor_pos - self.x >= self.width;
        if out_left || out_right {
            let _ = queue!(stdout(), cursor::Hide);
            return;
        }

        // Calculate cursor position considering Unicode character widths
        let line_start_char = self.code.line_to_char(self.r);
        let line_text = self.code.char_slice(line_start_char, line_start_char + self.c);
        let visual_cursor_pos = line_text.to_string().width();
        let cursor_x_pos = visual_cursor_pos + self.lp_width + line_number_digits - self.x;
        let cursor_y_pos = self.r - self.y;

        let _ = queue!(
            stdout(),
            cursor::MoveTo(cursor_x_pos as u16, cursor_y_pos as u16),
            FColor(Color::Reset),
            cursor::Show
        );

        stdout().flush().expect("flush");
    }

    fn draw_status(&mut self) {
        let status = self.status_line();
        let x = self.width - status.width();
        let y = self.height - 1;

        let _ = queue!(
            stdout(),
            cursor::Hide,
            cursor::MoveTo(x as u16, y as u16),
            FColor(self.scolor),
            Print(status)
        );
    }

    fn draw_run_button(&self, row: usize, color: Color) {
        let run = "▶";
        let _ = queue!(stdout(),
            cursor::Hide, cursor::MoveTo(self.lp_width as u16, row as u16),
            BColor(Color::Reset), FColor(color),
            Print(run),
            BColor(Color::Reset), FColor(Color::Reset)
        );
    }

    fn draw_logo(&mut self) {
        let logo = r#"🅡 🅔 🅓"#;
        // let logo = "RED";

        let lines:Vec<&str> = logo.split('\n').collect();
        // let logo_width = lines.get(0).unwrap().len();

        let fromy = self.height / 2 - lines.len() / 2;
        let fromx = self.lp_width + (self.width - self.lp_width)/ 2;

        let mut stdout = stdout();

        for r in 0..self.height{
            let _ = queue!(stdout,
                cursor::MoveTo(self.lp_width as u16, r as u16),
                terminal::Clear(ClearType::UntilNewLine)
            );
        }

        for (i,line) in lines.iter().enumerate() {
            let _ = queue!(stdout,
                cursor::MoveTo((fromx) as u16, (fromy + i) as u16),
                FColor(Color::Reset), Print(line)
            ).unwrap();
        }
    }

    fn focus_to_center(&mut self) {
        if self.r > self.height / 2 {
            self.y = self.r - (self.height / 2)
        }
    }

    fn fit_cursor(&mut self) {
        let len_lines = self.code.len_lines();
        if self.r >= len_lines {
            self.r = len_lines - 1;
        }

        let line_len = self.code.line_len(self.r);
        if self.c > line_len {
            self.c = line_len;
        }
    }

    fn handle_up(&mut self) {
        if self.r > 0 {
            self.r -= 1;
            self.fit_cursor();
            self.focus();
        }
    }

    fn handle_down(&mut self) {
        if self.r < self.code.len_lines() - 1 {
            self.r += 1;
            self.fit_cursor();
            self.focus();
        }
    }

    fn handle_page_up(&mut self) {
        if self.y > 0 {
            // Move view up by a page
            self.y = if self.y > self.height {
                self.y - self.height
            } else {
                0
            };
            // Move cursor up by a page
            self.r = if self.r > self.height {
                self.r - self.height
            } else {
                0
            };
            self.fit_cursor();
            self.upd = true;
            self.focus();
        }
    }

    fn handle_page_down(&mut self) {
        let max_y = if self.code.len_lines() > self.height {
            self.code.len_lines() - self.height
        } else {
            0
        };

        if self.y < max_y {
            // Move view down by a page
            self.y = if self.y + self.height < max_y {
                self.y + self.height
            } else {
                max_y
            };
            // Move cursor down by a page
            self.r = if self.r + self.height < self.code.len_lines() {
                self.r + self.height
            } else {
                self.code.len_lines() - 1
            };
            self.fit_cursor();
            self.upd = true;
            self.focus();
        }
    }

    fn handle_left(&mut self) {
        if self.c > 0 {
            self.c -= 1;
        } else if self.r > 0 {
            self.r -= 1;
            self.c = self.code.line_len(self.r);
        }

        self.upd = true;
        self.focus();
    }

    fn handle_right(&mut self) {
        if self.c < self.code.line_len(self.r) {
            self.c += 1;
        } else if self.r < self.code.len_lines() - 1 {
            self.r += 1;
            self.c = 0;
        }
        self.upd = true;
        self.focus();
    }

    fn handle_right_word(&mut self) {
        let line = self.code.line_at(self.r);
        if line.is_none() { return; }
        let line = line.unwrap();
        let next = utils::find_next_word(line, self.c+1);
        self.c = next;
        self.upd = true;
        self.focus();
    }

    fn handle_left_word(&mut self) {
        let line = self.code.line_at(self.r);
        if line.is_none() { return; }
        let line = line.unwrap();
        let next = utils::find_prev_word(line, self.c-1);
        self.c = next;
        self.upd = true;
        self.focus();
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

                self.c = indentation.width();
            },
            None => {},
        }
        self.focus();
        self.clean_diagnostics();
        self.reset_highlight_cache();
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

            let remove_all_indents = false;

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
            self.reset_highlight_cache();

            if remove_all_indents == false { return }
        }

        if self.c > 0 {
            // remove single char
            self.code.remove_char(self.r, self.c);

            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                lsp.lock().await.did_change(
                    self.r, self.c - 1, self.r,
                    self.c, &self.code.abs_path, ""
                ).await;
            }

            self.c -= 1;
            self.upd = true;
            self.clean_diagnostics();
            self.reset_highlight_cache();
        } else if self.r != 0 {
            // remove enter char
            let prev_line_len = self.code.line_len(self.r - 1);
            // self.code.remove_char(self.r, self.c);
            self.code.remove_text(self.r - 1, prev_line_len, self.r, self.c);

            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                lsp.lock().await.did_change(
                    self.r - 1, prev_line_len,
                    self.r, self.c, &self.code.abs_path, ""
                ).await;
            }

            self.r -= 1;
            self.c = prev_line_len;
            self.upd = true;
            self.clean_diagnostics();
            self.reset_highlight_cache();
        }

        self.focus();
    }

    fn copy_to_clipboard(&mut self, maybe_text: Option<String>) {

        let text = match maybe_text {
            Some(text) => text,
            None => {
                if self.selection.empty() { return; }
                let (y, x) = self.selection.from();
                let (yto, xto) = self.selection.to();
                let text = self.code.get_text(y, x, yto, xto);
                text
            },
        };

        let result = arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text.to_string()));

        if result.is_err() {
            self.clipboard = Some(text.to_string());
        }
    }

    fn get_clipboard(& self) -> anyhow::Result<String> {
        let maybe_text = arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .ok()
            .or_else(|| self.clipboard.clone());

        match maybe_text {
            Some(text) => Ok(text),
            None => Err(anyhow!("cant get clipboard")),
        }
    }

    async fn paste_from_clipboard(&mut self) {
        let text = match self.get_clipboard() {
            Ok(text) => text,
            Err(_) => return,
        };

        self.paste(text).await;
    }

    async fn paste(&mut self, text: String) {
        if text.is_empty() { return; }

        if self.selection.non_empty_and_active() {
            self.handle_cut().await;
        }

        self.code.insert_text(&text, self.r, self.c);

        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        if let Some(lsp) = self.lang2lsp.get(lang) {
            lsp.lock().await.did_change(
                self.r, self.c, self.r,
                self.c, &path, &text
            ).await;
        }

        for ch in text.chars() { match ch {
            '\n' => { self.r += 1; self.c = 0; }
            _ => self.c += ch.width().unwrap_or(1),
        }}

        self.upd = true;
        self.focus();
        self.clean_diagnostics();
        self.reset_highlight_cache();
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
                lsp.lock().await.did_change(
                    self.r, self.c, self.r, self.c, &path, &text
                ).await;
            }

            for ch in text.chars() { match ch {
                '\n' => { self.r += 1; self.c = 0; }
                _ => self.c += ch.width().unwrap_or(1),
            }}

            self.selection.clean();
            self.selection.keep_once = false;
            self.upd = true;
            self.focus();
            self.clean_diagnostics();
            self.reset_highlight_cache();

        } else if self.r < self.code.len_lines() - 1 {
            let text = self.code.get_text(self.r, 0, self.r + 1, 0);
            self.r += 1;
            self.code.insert_text(&text, self.r, 0);

            let path = &self.code.abs_path;
            let lang = &self.code.lang;

            if let Some(lsp) = self.lang2lsp.get(lang) {
                let change_text = format!("\n{}", &text);
                lsp.lock().await.did_change(
                    self.r-1, text.len(),
                    self.r-1, text.len(),
                    path, &change_text
                ).await;
            }

            self.upd = true;
            self.focus();
            self.clean_diagnostics();
            self.reset_highlight_cache();
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
        self.reset_highlight_cache();
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
        self.reset_highlight_cache();
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

    fn focus(&mut self) {
        let area = Rect::new(
            (self.lp_width) as u16, 0 as u16,
            self.width as u16, self.height as u16,
        );

        let width = area.width as usize;
        let height = area.height as usize;
        let total_lines = self.code.len_lines();
        let max_line_number = total_lines.max(1);
        let line_number_digits = max_line_number.to_string().len().max(5);

        let line = self.r;
        let col = self.c;

        let visible_width = width.saturating_sub(line_number_digits);
        let visible_height = height;

        if col < self.x {
            self.x = col;
            self.upd = true;
        } else if col >= self.x + visible_width {
            self.x = col.saturating_sub(visible_width - 1);
            self.upd = true;
        }

        if line < self.y {
            self.y = line;
            self.upd = true;
        } else if line >= self.y + visible_height {
            self.y = line.saturating_sub(visible_height - 1);
            self.upd = true;
        }
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
        self.focus();
        self.clean_diagnostics();
        self.reset_highlight_cache();
    }

    async fn insert_tab(&mut self) {
        let (r,c) = (self.r, self.c);
        let inserted = self.code.insert_tab(r,c);

        self.c += inserted.width();

        if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
            lsp.lock().await.did_change(r,c, r,c, &self.code.abs_path, &inserted).await;
        }
        self.upd = true;
        self.focus();
        self.clean_diagnostics();
        self.reset_highlight_cache();
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
        self.focus();
        self.handle_down();
        self.clean_diagnostics();
        self.reset_highlight_cache();
    }

    fn save(&mut self) {
        self.code.save_file().expect("Can not save file");
        self.upd = true;
        self.self_update = false;
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
                                _ => c = c.saturating_sub(1),
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
        self.focus();
        self.clean_diagnostics();
        self.reset_highlight_cache();
    }

    async fn redo(&mut self) {
        let maybe_change = self.code.redo();
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

                            self.c += 1;

                            if let Some(lsp) = self.lang2lsp.get(&self.code.lang) {
                                lsp.lock().await.did_change(
                                    r, c, r_end, c_end, &self.code.abs_path, &change.text
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
                                    r, c, r, c, &self.code.abs_path, ""
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
        self.focus();
        self.clean_diagnostics();
        self.reset_highlight_cache();
    }

    fn update_search_results(&mut self) {
        if self.search.pattern.len_chars() > 0 {
            let search_results = self.code.search(
                &self.search.pattern.to_string()
            );

            self.search.results = search_results
                .iter()
                .map(|(line, column)| SearchResult {
                    line: *line,
                    column: *column,
                    preview: None,
                })
                .collect();

            let closest_to_cursor = self.search.results
                .iter()
                .enumerate()
                .min_by_key(|(_, result)| {
                    let dy = result.line.abs_diff(self.r);
                    let dx = result.column.abs_diff(self.c);
                    dy * 1000 + dx
                })
                .map(|(i, _)| i);

            self.search.index = closest_to_cursor.unwrap_or(0);

        } else {
            self.search.results.clear();
            self.search.index = 0;
        }
    }

    pub async fn handle_local_search(&mut self) {
        let saved_r = self.r.clone();
        let saved_c = self.c.clone();
        let saved_selection = self.selection.clone();

        let mut end = false;
        let mut changed = false;

        self.search.active = true;

        if self.selection.non_empty_and_active() {
            let (y, x) = self.selection.from();
            let (yto, xto) = self.selection.to();
            let selected_text = self.code.get_text(y, x, yto, xto);
            self.search.pattern = ropey::Rope::from_str(&selected_text);
            self.search.cursor_pos = self.search.pattern.len_chars();
            self.update_search_results();
            changed = true;
        } else if self.search.pattern.len_chars() > 0 {
            self.search.cursor_pos = self.search.pattern.len_chars();
            self.update_search_results();
            changed = true;
        }

        let mut reader = EventStream::new();

        while !end {
            self.draw_search_line(self.search.cursor_pos, self.height - 1);

            if changed && self.search.pattern.len_chars() > 0 && !self.search.results.is_empty() {
                let search_result = &self.search.results[self.search.index];
                let sy = search_result.line;
                let sx = search_result.column;
                self.r = sy;
                self.c = sx + self.search.pattern.to_string().width();
                self.focus();
                if self.r - self.y == self.height - 1 {
                    self.y += 1;
                }
                self.selection.active = true;
                self.selection.set_start(sy, sx);
                self.selection.set_end(sy, sx + self.search.pattern.to_string().width());
                self.upd = true;
                self.draw().await;
                self.draw_search_line(self.search.cursor_pos, self.height - 1);
                changed = false;
            }

            if changed && self.search.pattern.len_chars() > 0 && self.search.results.is_empty() {
                self.selection.active = false;
                self.upd = true;
                self.draw().await;
                self.draw_search_line(self.search.cursor_pos, self.height - 1);
                changed = false;
            }

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            match event {
                                Event::Key(KeyEvent { code: KeyCode::Esc, .. }) => {
                                    self.r = saved_r; self.c = saved_c;
                                    self.selection = saved_selection.clone();
                                    self.upd = true;
                                    if self.code.file_name.is_empty() {
                                        let _ = queue!(stdout(), terminal::Clear(ClearType::All));
                                        stdout().flush().ok();
                                    }
                                    end = true;
                                }
                                _ => {
                                    end = self.handle_search_event(event).await;
                                    changed = true;
                                }
                            }
                        }
                        Some(Err(e)) => {
                            debug!("Error reading event: {:?}", e);
                            end = true;
                        }
                        None => {
                            end = true;
                        }
                    }
                }
            }
        }

        self.upd = true;
        self.search.active = false;
    }

    async fn handle_search_event(&mut self, event: Event) -> bool {
        // Returns true if the search should end
        match event {
            Event::Resize(w, h) => {
                self.upd = true;
                self.tree_view.upd = true;
                self.resize(w as usize, h as usize);
                self.draw().await;
                false
            }
            Event::Key(key_event) => {

                if key_event.modifiers == KeyModifiers::CONTROL
                    && key_event.code == KeyCode::Char('v') {
                        if let Ok(pasted_text) = self.get_clipboard() {
                            self.clean_search_line();
                            for ch in pasted_text.chars() {
                                self.search.pattern.insert_char(self.search.cursor_pos, ch);
                                self.search.cursor_pos += 1;
                            }
                            self.update_search_results();
                        }
                    return false;
                }

                if key_event.modifiers == KeyModifiers::CONTROL
                    && key_event.code == KeyCode::Char('g') {
                    self.hanle_global_search().await;
                    self.overlay_lines.clear();
                    return true;
                }

                match key_event.code {
                    KeyCode::Up => {
                        if self.search.index > 0 {
                            self.search.index -= 1;
                        } else if !self.search.results.is_empty() {
                            self.search.index = self.search.results.len() - 1;
                        }
                        self.upd = true;
                        false
                    }
                    KeyCode::Down => {
                        self.search.index += 1;
                        if self.search.index >= self.search.results.len() {
                            self.search.index = 0;
                        }
                        self.upd = true;
                        false
                    }
                    KeyCode::Left => {
                        if self.search.cursor_pos > 0 {
                            self.search.cursor_pos -= 1;
                        }
                        false
                    }
                    KeyCode::Right => {
                        if self.search.cursor_pos < self.search.pattern.len_chars() {
                            self.search.cursor_pos += 1;
                        }
                        false
                    }
                    KeyCode::Enter => {
                        if self.code.file_name.is_empty() {
                            self.hanle_global_search().await;
                            self.overlay_lines.clear();
                            self.upd = true;
                            if self.code.file_name.is_empty() {
                                let _ = queue!(stdout(), terminal::Clear(ClearType::All));
                                stdout().flush().ok();
                            }
                        }
                        true
                    }
                    KeyCode::Esc => true,
                    KeyCode::Backspace => {
                        if self.search.cursor_pos > 0 {
                            self.search.cursor_pos -= 1;
                            self.clean_search_line();
                            self.search.pattern.remove(self.search.cursor_pos..self.search.cursor_pos + 1);
                            self.update_search_results();
                        }
                        false
                    }
                    KeyCode::Char(c) => {
                        self.clean_search_line();
                        self.search.pattern.insert_char(self.search.cursor_pos, c);
                        self.search.cursor_pos += 1;
                        self.update_search_results();
                        false
                    }
                    _ => {
                        debug!("Unhandled key code: {:?}", key_event.code);
                        false
                    }
                }
            }
            _ => false,
        }
    }

    pub fn draw_search_line(&mut self, x:usize, y:usize) {
        let prefix = "search: ";
        let space = " ".repeat(10);
        let line = if !self.search.results.is_empty() && self.search.pattern.len_chars() > 0 {
            let postfix = format!("{}/{}", self.search.index+1, self.search.results.len());
            format!("{}{} {}{}", prefix, &self.search.pattern, postfix, space)
        } else {
            format!("{}{} {}", prefix, &self.search.pattern, space)
        };

        let _ = queue!(stdout(),
            cursor::MoveTo((self.lp_width) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(line),
        );
        let _ = queue!(stdout(),
            cursor::MoveTo((self.lp_width + prefix.len() + x) as u16, y as u16),
        );

        let _ = stdout().flush();
    }

    pub fn clean_search_line(&mut self) {
        let prefix = "search: ";
        let line = if !self.search.results.is_empty() && self.search.pattern.len_chars() > 0 {
            let postfix = format!("{}/{}", self.search.index+1, self.search.results.len());
            format!("{}{} {}", prefix, &self.search.pattern, postfix)
        } else {
            format!("{}{}", prefix, &self.search.pattern)
        };

        let _ = queue!(stdout(),
            cursor::MoveTo((self.lp_width + 1) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(" ".repeat(line.to_string().width())),
        );

        let _ = stdout().flush();
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

    pub async fn lsp_update(&mut self) {
        let lang = self.code.lang.clone();
        let lsp = self.lang2lsp.get(&lang);
        match lsp {
            Some(lsp) => {
                let mut lsp = lsp.lock().await;
                let file_content = self.code.text.to_string();
                lsp.did_open(&self.code.lang, &self.code.abs_path, &file_content);
            },
            None => {},
        }
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

    pub async fn completion(&mut self) {

        loop {
            let mut changed = false;

            let path = &self.code.abs_path;
            let lang = &self.code.lang;

            let completion_result = match self.lang2lsp.get(lang) {
                Some(lsp) => lsp.lock().await.completion(&path, self.r, self.c).await,
                None => return,
            };

            let mut completion_result = match completion_result {
                Ok(c) => c, Err(_) => return,
            };

            if completion_result.is_empty() { return; }

            self.set_lsp_status("lsp completion").await;

            let (mut selected, mut selected_offset) = (0, 0);
            let height = 5;

            let line = match self.code.line_at(self.r) {
                Some(line) => line, None => return,
            };

            let prev = utils::find_prev_word(line, self.c);
            let prev_word = line.chars().skip(prev).take(self.c - prev).collect::<String>();

            // Sort completion items by matches score
            completion_result.sort_by(|a, b| {
                let sa = score_matches(&a.label, &prev_word);
                let sb = score_matches(&b.label, &prev_word);
                let r = sb.cmp(&sa);
                if r == Ordering::Equal {
                    a.label.len().cmp(&b.label.len())
                } else { r }
            });

            let options = &completion_result;

            while !changed {
                // calculate scrolling offsets
                if selected < selected_offset { selected_offset = selected }
                if selected >= selected_offset + height { selected_offset = selected - height + 1 }

                self.draw_completion(height, options, selected, selected_offset);

                let mut reader = EventStream::new();
                let mut event = reader.next().fuse();

                select! {
                    maybe_event = event => {
                        changed = false;
                        match maybe_event {
                            Some(Ok(event)) => {
                                if event == Event::Key(KeyCode::Esc.into()) {
                                    self.upd = true;
                                    return;
                                }
                                if event == Event::Key(KeyCode::Down.into())
                                    && selected < options.len() - 1 {
                                    selected += 1;
                                }
                                if event == Event::Key(KeyCode::Up.into())
                                    && selected > 0 {
                                    selected -= 1;
                                }
                                if event == Event::Key(KeyCode::Enter.into())
                                    || event == Event::Key(KeyCode::Tab.into()) {
                                    let item = completion_result.get(selected).unwrap();
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
                                            KeyCode::Char(' ') => {
                                                self.upd = true;
                                                return;
                                            }
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
                            }
                            Some(Err(e)) => {debug!("Error: {:?}\r", e) },
                            None => break,
                        }
                    }
                };
            }
        }
    }

    pub fn draw_completion(
        &mut self, height: usize, options: &Vec<lsp_types::CompletionItem>, selected: usize, offset: usize,
    ) {
        let max_height: usize = options.len().min(height);
        let max_width: usize = 30;

        let ln_width = self.get_line_number_width();
        let word_offset = self.code.offset(self.r, self.c);
        let (word_start, _) = self.code.word_boundaries(word_offset);
        let (_, word_start_col) = self.code.point(word_start);

        let max_label_width = options.iter().map(|o| o.label.len()).max().unwrap_or(max_width);

        let cursor_screen_row = self.r - self.y;
        let available_below = self.height.saturating_sub(cursor_screen_row + 1);

        let visible_height = options.len().min(max_height);

        let draw_above = available_below < max_height
            && cursor_screen_row >= max_height;

        let from_y = if draw_above {
            cursor_screen_row.saturating_sub(visible_height)
        } else {
            cursor_screen_row + 1
        };

        for row in 0..visible_height {
            let i = row + offset;
            if i >= options.len() {
                break;
            }

            let option = &options[i];
            let is_selected = selected == i;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let limit = self.width.saturating_sub(self.lp_width + ln_width + word_start_col);
            let label = format!(" {:width$} ", option.label, width = max_label_width)
                .chars()
                .take(limit)
                .collect::<String>();

            let draw_row = from_y + row;
            let draw_col = self.lp_width + ln_width + word_start_col - 1;

            let _ = queue!(
                stdout(),
                cursor::MoveTo(draw_col as u16, draw_row as u16),
                BColor(bgcolor),
                FColor(self.lncolor),
                Print(label),
                BColor(Color::Reset),
                FColor(Color::Reset),
            );
        }

        self.draw_cursor();
    }

    pub async fn lsp_completion_apply(
        &mut self, item: &lsp_types::CompletionItem
    ) {
        if item.text_edit.is_none() && item.label.is_empty() { return; }

        let line = match self.code.line_at(self.r) {
            Some(line) => line, None => return,
        };

        let prev = utils::find_prev_word(line, self.c);
        let next = utils::find_next_word(line, self.c);

        let insert_text = match item.text_edit.as_ref() {
            Some(lsp_types::CompletionTextEdit::InsertAndReplace(t)) => &t.new_text,
            Some(lsp_types::CompletionTextEdit::Edit(t)) => &t.new_text,
            _ => &item.label,
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
        self.reset_highlight_cache();
    }

    async fn definition(&mut self) {
        let path = &self.code.abs_path;
        let lang = &self.code.lang;

        let definition_result = match self.lang2lsp.get(lang) {
            Some(lsp) => lsp.lock().await.definition(&path, self.r, self.c).await,
            None => { return },
        };

        let definition = match &definition_result {
            Ok(def) if def.len() == 1 => &def[0],
            _ => return,
        };

        self.save_cursor_to_history();

        if definition.uri.to_string() != format!("file://{}", self.code.abs_path) {
            let path = definition.uri.to_string().split("file://").nth(1).unwrap().to_string();
            self.open_file(&path).await;
        }

        if definition.range.start.line as usize > self.code.len_lines() ||
            definition.range.start.character as usize >
                self.code.line_len(definition.range.start.line as usize) {
            return;
        }

        self.r = definition.range.start.line as usize;
        self.c = definition.range.start.character as usize;
        self.focus();
        self.save_cursor_to_history();

        self.upd = true;
        self.tree_view.upd = true;
    }

    pub async fn references(&mut self) {
        let saved_r = self.r.clone();
        let saved_c = self.c.clone();
        let saved_y = self.y.clone();
        let saved_x = self.x.clone();
        let saved_path = self.code.abs_path.clone();
        self.save_cursor_to_history();

        loop {
            let start = Instant::now();

            let references_result = match self.lang2lsp.get(&self.code.lang) {
                Some(lsp) => lsp.lock().await.references(&self.code.abs_path, self.r, self.c).await,
                None => return,
            };

            let elapsed = start.elapsed().as_millis();

            let references = match references_result {
                Ok(refr) if !refr.is_empty() => refr,
                _ => return,
            };

            if references.len() == 0 { return; }
            if references.len() == 1 { self.apply_reference(&references[0]).await; return; }

            let max_visible = 3;
            let (mut selected, mut selected_offset) = (0, 0);
            let (height, width) = (max_visible, 30);
            self.upd = true; self.tree_view.upd = true;

            self.overlay_lines.clear();

            let mut reader = EventStream::new();

            loop {

                if selected < selected_offset { selected_offset = selected } // calculate scrolling offsets
                if selected >= selected_offset + height { selected_offset = selected - height + 1 }

                let reference = references.get(selected).unwrap();

                if reference.uri.to_string() != format!("file://{}", &self.code.abs_path) {
                    let path = reference.uri.to_string().split("file://").nth(1).unwrap().to_string();
                    self.open_file(&path).await;
                }

                self.r = reference.range.start.line as usize;
                self.c = reference.range.start.character as usize;
                self.focus();
                self.focus_to_center();
                self.selection.set_start(reference.range.start.line as usize, reference.range.start.character as usize);
                self.selection.set_end(reference.range.end.line as usize, reference.range.end.character as usize);
                self.selection.activate();

                let count = std::cmp::min(max_visible, references.len());
                let fromy = self.height - count - 1;
                for i in fromy..=self.height { self.overlay_lines.insert(i); }

                self.draw().await;
                self.draw_references(height, width, fromy, &references, selected, selected_offset, elapsed);
                self.draw_cursor();

                let mut event = reader.next().fuse();

                select! {
                    maybe_event = event => {
                        match maybe_event {
                            Some(Ok(event)) => {
                                if event == Event::Key(KeyCode::Esc.into()) {
                                    if self.code.abs_path != saved_path {
                                        self.open_file(&saved_path).await;
                                    }
                                    self.r = saved_r; self.c = saved_c;
                                    self.y = saved_y; self.x = saved_x;
                                    self.focus();
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
                                    self.selection.clean();
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

    async fn apply_reference(&mut self, reference: &lsp_types::Location) {
        self.save_cursor_to_history();
        if reference.uri.to_string() != format!("file://{}", self.code.abs_path) {
            let path = reference.uri.to_string().split("file://").nth(1).unwrap().to_string();
            self.open_file(&path).await;
        }
        self.r = reference.range.start.line as usize;
        self.c = reference.range.start.character as usize;
        self.focus();
        self.save_cursor_to_history();
        self.upd = true;
        self.tree_view.upd = true;
    }

    pub fn draw_references(
        &mut self,
        height: usize, width:usize, fromy:usize,
        options: &Vec<lsp_types::Location>,
        selected: usize, offset: usize, elapsed:u128
    ) {
        let options: Vec<String> = options.iter().enumerate().map(|(i, reff)| {
            format!(
                "{}/{} {}:{} {}", i+1, options.len(), reff.uri.to_string().split("file://").nth(1).unwrap(),
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

            let _ = queue!(stdout(),
                cursor::MoveTo(self.lp_width as u16, (row + fromy) as u16),
                BColor(bgcolor), FColor(self.lncolor), Print(label),  BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        let status = format!("lsp references, elapsed {} ms {}", elapsed, " ".repeat(10));

        let _ = queue!(stdout(),
            cursor::MoveTo((self.lp_width) as u16, (self.height-1) as u16),
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
            Ok(hr) => hr, _ => return,
        };

        self.set_lsp_status("lsp completion").await;

        let (end, mut selected, mut selected_offset) = (false, 0, 0);
        let height = 10;

        let mut reader = EventStream::new();

        while !end {
            // calculate scrolling offsets
            if selected < selected_offset { selected_offset = selected }
            if selected >= selected_offset + height { selected_offset = selected - height + 1 }

            // The original code tried to split hover_result.contents directly, which is an enum, not a string.
            // Instead, we first extract the string value(s) from hover_result.contents, then split into lines.

            let value: String = match &hover_result.contents {
                lsp_types::HoverContents::Scalar(marked_string) => {
                    match marked_string {
                        lsp_types::MarkedString::String(s) => s.clone(),
                        lsp_types::MarkedString::LanguageString(ls) => ls.value.clone(),
                    }
                },
                lsp_types::HoverContents::Array(marked_strings) => {
                    marked_strings.iter().map(|marked_string| {
                        match marked_string {
                            lsp_types::MarkedString::String(s) => s.clone(),
                            lsp_types::MarkedString::LanguageString(ls) => ls.value.clone(),
                        }
                    }).collect::<Vec<String>>().join("\n")
                },
                lsp_types::HoverContents::Markup(markup_content) => markup_content.value.clone(),
            };

            let options: Vec<String> = value.lines().map(|s| s.to_string()).collect();

            if options.is_empty() { return }

            self.draw_hover(height, &options, selected, selected_offset);

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if event == Event::Key(KeyCode::Esc.into()) {
                                self.upd = true;
                                self.tree_view.upd = true;
                                // self.clear_all();
                                return ;
                            }
                            if event == Event::Key(KeyCode::Down.into())
                                && selected < options.len() - 1 {
                                selected += 1;
                            }
                            if event == Event::Key(KeyCode::Up.into())
                                && selected > 0 {
                                selected -= 1;
                            }
                            if event == Event::Key(KeyCode::Enter.into())
                                || event == Event::Key(KeyCode::Tab.into()) {
                                self.upd = true;
                                self.tree_view.upd = true;
                                // self.clear_all();
                                return ;
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

    pub fn draw_hover(
        &mut self,
        height: usize,
        options: &Vec<String>,
        selected: usize,
        offset: usize,
    ) {
        let max_height: usize = options.len().min(height);
        let max_width: usize = 80;

        let ln_width = self.get_line_number_width();
        let word_offset = self.code.offset(self.r, self.c);
        let (word_start, _) = self.code.word_boundaries(word_offset);
        let (_, word_start_col) = self.code.point(word_start);

        let max_label_width = options.iter().map(|s| s.len()).max().unwrap_or(max_width);

        let cursor_screen_row = self.r - self.y;
        let available_below = self.height.saturating_sub(cursor_screen_row + 1);

        let visible_height = options.len().min(max_height);

        let draw_above = available_below < max_height
            && cursor_screen_row >= max_height;

        let from_y = if draw_above {
            cursor_screen_row.saturating_sub(visible_height)
        } else {
            cursor_screen_row + 1
        };

        for row in 0..visible_height {
            let i = row + offset;
            if i >= options.len() {
                break;
            }

            let option = &options[i];
            let is_selected = selected == i;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let limit = self.width.saturating_sub(self.lp_width + ln_width + word_start_col);
            let label = format!(" {:width$} ", option, width = max_label_width)
                .chars()
                .take(limit)
                .collect::<String>();

            let draw_row = from_y + row;
            let draw_col = self.lp_width + ln_width + word_start_col - 1;

            let _ = queue!(
                stdout(),
                cursor::MoveTo(draw_col as u16, draw_row as u16),
                BColor(bgcolor),
                FColor(self.lncolor),
                Print(label),
                BColor(Color::Reset),
                FColor(Color::Reset),
            );
        }

        self.draw_cursor();
        stdout().flush().expect("cant flush");
    }

    pub async fn handle_errors(&mut self) {
        let saved_r = self.r.clone();
        let saved_c = self.c.clone();
        let saved_path = self.code.abs_path.clone();

        let (mut selected, mut selected_offset) = (0, 0);
        let (height, width) = (3, 30);
        self.upd = true; self.tree_view.upd = true;

        let uri = format!("file://{}", self.code.abs_path);

        let diagnostics: Vec<lsp_types::Diagnostic> = {
            let diagnostics = self.diagnostics.clone();
            let maybe_diagnostics = diagnostics.lock().await;
            let maybe_diagnostics = maybe_diagnostics.get(&uri);

            let diagnostics: Vec<lsp_types::Diagnostic> = match maybe_diagnostics {
                Some(d) => d.diagnostics.iter()
                    // .filter(|d| d.severity == 1)
                    .map(|d|d.clone()).collect(),
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

            self.r = diagnostic.range.start.line as usize;
            self.c = diagnostic.range.start.character as usize;

            let fromy = self.height.saturating_sub(std::cmp::min(height, diagnostics.len()));
            for i in fromy.saturating_sub(1)..=self.height { self.overlay_lines.insert(i); }

            self.focus();
            self.focus_to_center();
            self.draw().await;
            self.draw_errors(height, width, fromy-1, &diagnostics, selected, selected_offset);
            self.draw_cursor();

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if event == Event::Key(KeyCode::Enter.into()) {
                                if self.code.abs_path != saved_path {
                                    self.open_file(&saved_path).await;
                                }
                                self.focus();
                                self.selection.clean();
                                self.upd = true;
                                self.tree_view.upd = true;
                                self.overlay_lines.clear();
                                self.clear_all();
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
                            if event == Event::Key(KeyCode::Char('c').into()) {
                                let error = &diagnostics[selected];
                                self.copy_to_clipboard(Some(error.message.clone()));
                                return;
                            }

                            if let Event::Resize(w, h) = event {
                                self.upd = true;
                                self.tree_view.upd = true;
                                self.resize(w as usize, h as usize);
                                self.draw().await;
                            }

                            if event == Event::Key(KeyCode::Esc.into()){
                                self.r = saved_r; self.c = saved_c; // restore cursor
                                self.upd = true;
                                self.tree_view.upd = true;
                                self.overlay_lines.clear();
                                self.clear_all();
                                return;
                            }
                        }
                        Some(Err(e)) => { debug!("Error: {:?}\r", e); return; },
                        None => break,
                    }
                }
            };
        }

        self.overlay_lines.clear();
        self.clear_all();
        self.upd = true;
        self.tree_view.upd = true;
    }

    pub fn draw_errors(
        &mut self,
        height: usize, _width: usize, fromy: usize,
        options: &Vec<lsp_types::Diagnostic>,
        selected: usize, offset: usize
    ) {
        let limit = self.width - self.lp_width - 1;

        let options: Vec<String> = options.iter().enumerate().map(|(i, diagnostic)| {
            let prefix = format!("{}/{} {}:{} ", i+1, options.len(),
                diagnostic.range.start.line,
                diagnostic.range.start.character,
            );
            let message: String = diagnostic.message.chars().take(limit-prefix.len()).collect();
            format!("{}{}", prefix, message)
        }).collect();

        for row in 0..options.len() {
            if row >= options.len() || row >= height { break; }
            let option = &options[row + offset];
            let message = option.replace("\n", " ").chars().take(limit).collect::<String>();

            let is_selected = selected == row + offset;
            let bgcolor = if is_selected { Color::Grey } else { Color::Reset };

            let _ = queue!(stdout(),
                cursor::MoveTo((self.lp_width) as u16, (row + fromy) as u16),
                BColor(bgcolor), FColor(self.lncolor), Print(message),
                terminal::Clear(ClearType::UntilNewLine), BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        let status = format!("Found {} problems {}", options.len(), " ".repeat(20));
        let _ = queue!(stdout(),
            cursor::MoveTo((self.lp_width) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(status),
        );
    }

    fn global_search(& self, pattern: &str) -> Vec<(String, SearchResult)> {
        match search_in_directory(Path::new("./"), pattern) {
            Ok(results) => results.into_iter()
                .flat_map(|sr| {
                    let path = sr.file_path;
                    sr.search_results.into_iter().map(move |r| (path.clone(), r))
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    pub async fn hanle_global_search(&mut self) {
        if self.search.pattern.len_chars() == 0 { return }

        let saved_r = self.r.clone();
        let saved_c = self.c.clone();
        let saved_selection = self.selection.clone();
        let saved_path = self.code.abs_path.clone();

        let max_visible = 3;
        let mut changed = true;
        let (mut selected, mut selected_offset) = (0, 0);
        self.upd = true; self.tree_view.upd = true;

        self.overlay_lines.clear();

        let start = Instant::now();
        let search_results = self.global_search(&self.search.pattern.to_string());
        if search_results.is_empty() { return }
        let elapsed = start.elapsed().as_millis();

        let height = max_visible.min(search_results.len());
        let width = self.width - self.lp_width - 1;

        let mut reader = EventStream::new();

        loop {

            if selected < selected_offset { selected_offset = selected } // calculate scrolling offsets
            if selected >= selected_offset + height { selected_offset = selected - height + 1 }

            if changed {
                self.upd = true;
                self.tree_view.upd = true;

                let search_result = search_results.get(selected).unwrap();

                if search_result.0 != self.code.abs_path {
                    self.open_file(&search_result.0).await;
                }

                self.r = search_result.1.line-1;
                self.c = search_result.1.column;
                self.focus();
                self.focus_to_center();
                self.selection.set_start(search_result.1.line-1, search_result.1.column);
                let pattern_len = self.search.pattern.to_string().width();
                self.selection.set_end(search_result.1.line-1, search_result.1.column + pattern_len);
                self.selection.activate();

                let fromy = self.height.saturating_sub(max_visible.min(search_results.len()));
                for i in fromy.saturating_sub(1)..=self.height { self.overlay_lines.insert(i); }

                self.draw().await;
                self.draw_global_search_result(
                    height, width, fromy-1, &search_results, selected, selected_offset, elapsed
                );
                self.draw_cursor();
                changed = false;
            }

            let mut event = reader.next().fuse();

            select! {
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if event == Event::Key(KeyCode::Esc.into()) {
                                if self.code.abs_path != saved_path {
                                    self.open_file(&saved_path).await;
                                }
                                self.r = saved_r; self.c = saved_c;
                                self.selection = saved_selection;
                                self.focus();
                                self.selection.clean();

                                self.upd = true;
                                self.tree_view.upd = true;
                                self.clear_all();
                                return;
                            }
                            if event == Event::Key(KeyCode::Down.into())
                                && selected < search_results.len() - 1 {
                                selected += 1;
                                changed = true;
                            }

                            if event == Event::Key(KeyCode::Up.into()) && selected > 0 {
                                selected -= 1;
                                changed = true;
                            }

                            if let Event::Resize(w, h) = event {
                                self.resize(w as usize, h as usize);
                                changed = true;
                            }

                            if event == Event::Key(KeyCode::Enter.into())
                                || event == Event::Key(KeyCode::Tab.into()) {
                                self.clear_all();
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

    pub fn draw_global_search_result(&mut self,
        height: usize, width:usize, fromy: usize,
        options: &Vec<(String, SearchResult)>,
        selected: usize, offset: usize, elapsed: u128
    ) {
        let limit = self.width - self.lp_width - 1;

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

            let label = format!("{:width$} ", option, width = width);

            let _ = queue!(stdout(),
                cursor::MoveTo((self.lp_width) as u16, (row + fromy) as u16),
                BColor(bgcolor), FColor(self.lncolor), Print(label),
                terminal::Clear(ClearType::UntilNewLine), BColor(Color::Reset), FColor(Color::Reset),
            );
        }

        let status = format!("global search on '{}', elapsed {} ms {}",
            &self.search.pattern, elapsed, " ".repeat(20)
        );

        let _ = queue!(stdout(),
            cursor::MoveTo((self.lp_width) as u16, (self.height-1) as u16),
            BColor(Color::Reset), FColor(Color::Reset), Print(status),
        );

        stdout().flush().expect("cant flush");
    }

    pub fn save_cursor_to_history(&mut self) {
        if self.code.abs_path.is_empty() { return }

        let cp = CursorPosition {
            filename: self.code.abs_path.clone(),
            row: self.r.clone(),
            col: self.c.clone(),
            y: self.y.clone(),
            x: self.x.clone(),
        };
        self.cursor_history.push(cp);
    }

    async fn undo_cursor(&mut self) {
        if let Some(prev) = self.cursor_history.current() {
            if prev.row == self.r && prev.col == self.c && prev.filename == self.code.abs_path {
                let _ = self.cursor_history.undo();
            }
        }

        if let Some(history) = self.cursor_history.undo() {
            if history.filename != self.code.abs_path {
                self.open_file(&history.filename).await;
            }

            self.r = history.row;
            self.c = history.col;
            self.y = history.y;
            self.x = history.x;
            self.upd = true;
            self.fit_cursor();
            self.focus();
        }
    }

    async fn redo_cursor(&mut self) {
        if let Some(next) = self.cursor_history.peek_redo() {
            if next.row == self.r && next.col == self.c && next.filename == self.code.abs_path {
                let _ = self.cursor_history.redo();
            }
        }

        if let Some(history) = self.cursor_history.redo() {
            if history.filename != self.code.abs_path {
                self.open_file(&history.filename).await;
            }

            self.r = history.row;
            self.c = history.col;
            self.y = history.y;
            self.x = history.x;
            self.upd = true;
            self.fit_cursor();
            self.focus();
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
        self.reset_highlight_cache();
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
        Self::deinit().expect("Error deinit")
    }
}
