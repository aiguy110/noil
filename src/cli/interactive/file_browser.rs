use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use std::fs;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};

/// Guard that restores terminal state on drop (even on panic/Ctrl+C).
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

struct DirEntry {
    name: String,
    is_dir: bool,
    size: Option<u64>,
}

struct FileBrowserState {
    current_dir: PathBuf,
    entries: Vec<DirEntry>,
    list_state: ListState,
    filter: String,
    filtering: bool,
}

impl FileBrowserState {
    fn new(start_dir: &Path) -> io::Result<Self> {
        let current_dir = start_dir.canonicalize()?;
        let mut state = Self {
            current_dir,
            entries: Vec::new(),
            list_state: ListState::default(),
            filter: String::new(),
            filtering: false,
        };
        state.refresh_entries()?;
        Ok(state)
    }

    fn refresh_entries(&mut self) -> io::Result<()> {
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        let read_dir = fs::read_dir(&self.current_dir)?;
        for entry in read_dir.flatten() {
            let metadata = entry.metadata();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = metadata.as_ref().ok().and_then(|m| {
                if m.is_file() {
                    Some(m.len())
                } else {
                    None
                }
            });
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files
            if name.starts_with('.') {
                continue;
            }

            let entry = DirEntry { name, is_dir, size };
            if is_dir {
                dirs.push(entry);
            } else {
                files.push(entry);
            }
        }

        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        self.entries.clear();
        // Parent directory always first
        self.entries.push(DirEntry {
            name: "../".to_string(),
            is_dir: true,
            size: None,
        });
        self.entries.extend(dirs);
        self.entries.extend(files);

        self.list_state.select(Some(0));
        Ok(())
    }

    fn filtered_entries(&self) -> Vec<(usize, &DirEntry)> {
        if self.filter.is_empty() {
            self.entries.iter().enumerate().collect()
        } else {
            let lower_filter = self.filter.to_lowercase();
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.name == "../" || e.name.to_lowercase().contains(&lower_filter)
                })
                .collect()
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let filtered = self.filtered_entries();
        if filtered.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let new = (current + delta).clamp(0, filtered.len() as i32 - 1) as usize;
        self.list_state.select(Some(new));
    }

    fn go_to_parent(&mut self) -> io::Result<()> {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.filter.clear();
            self.filtering = false;
            self.refresh_entries()?;
        }
        Ok(())
    }

    fn enter_selected(&mut self) -> io::Result<Option<PathBuf>> {
        let filtered = self.filtered_entries();
        let idx = match self.list_state.selected() {
            Some(i) => i,
            None => return Ok(None),
        };
        let (orig_idx, _) = match filtered.get(idx) {
            Some(e) => *e,
            None => return Ok(None),
        };
        let entry = &self.entries[orig_idx];

        if entry.is_dir {
            if entry.name == "../" {
                self.go_to_parent()?;
            } else {
                self.current_dir = self.current_dir.join(&entry.name);
                self.filter.clear();
                self.filtering = false;
                self.refresh_entries()?;
            }
            Ok(None)
        } else {
            // File selected
            Ok(Some(self.current_dir.join(&entry.name)))
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Launch a file browser TUI and return the selected file path, or None if cancelled.
pub fn browse_for_file() -> io::Result<Option<PathBuf>> {
    let cwd = std::env::current_dir()?;
    let mut guard = TerminalGuard::new()?;
    let mut state = FileBrowserState::new(&cwd)?;

    loop {
        guard.terminal.draw(|frame| {
            let area = frame.area();

            let chunks = Layout::vertical([
                Constraint::Length(3), // header + filter
                Constraint::Min(3),   // file list
                Constraint::Length(2), // help
            ])
            .split(area);

            // Header
            draw_header(frame, chunks[0], &state);

            // File list
            draw_file_list(frame, chunks[1], &mut state);

            // Help bar
            draw_help(frame, chunks[2], &state);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Ctrl+C always quits
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(None);
            }

            if state.filtering {
                match key.code {
                    KeyCode::Esc => {
                        state.filter.clear();
                        state.filtering = false;
                        state.list_state.select(Some(0));
                    }
                    KeyCode::Enter => {
                        state.filtering = false;
                    }
                    KeyCode::Backspace => {
                        state.filter.pop();
                        state.list_state.select(Some(0));
                    }
                    KeyCode::Char(c) => {
                        state.filter.push(c);
                        state.list_state.select(Some(0));
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        return Ok(None);
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        state.move_selection(1);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        state.move_selection(-1);
                    }
                    KeyCode::Char('g') => {
                        state.list_state.select(Some(0));
                    }
                    KeyCode::Char('G') => {
                        let len = state.filtered_entries().len();
                        if len > 0 {
                            state.list_state.select(Some(len - 1));
                        }
                    }
                    KeyCode::Char('/') => {
                        state.filtering = true;
                        state.filter.clear();
                    }
                    KeyCode::Char('-') | KeyCode::Backspace => {
                        state.go_to_parent()?;
                    }
                    KeyCode::Enter => {
                        if let Some(path) = state.enter_selected()? {
                            return Ok(Some(path));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &FileBrowserState) {
    let header_text = format!(" Browse: {}", state.current_dir.display());

    let filter_line = if state.filtering || !state.filter.is_empty() {
        format!(" Filter: {}", state.filter)
    } else {
        String::new()
    };

    let lines = vec![
        Line::from(Span::styled(
            header_text,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            filter_line,
            Style::default().fg(Color::Yellow),
        )),
    ];

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .title("Select a log file");
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_file_list(frame: &mut ratatui::Frame, area: Rect, state: &mut FileBrowserState) {
    let filtered = state.filtered_entries();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|(_, entry)| {
            let (suffix, style) = if entry.is_dir {
                (
                    "<DIR>".to_string(),
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                let size_str = entry.size.map(format_size).unwrap_or_default();
                (size_str, Style::default())
            };

            // Pad name to align size column
            let display_name = if entry.is_dir && !entry.name.ends_with('/') {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };

            let width = area.width.saturating_sub(4) as usize;
            let name_width = width.saturating_sub(suffix.len() + 2);
            let line = format!("{:<name_width$}  {}", display_name, suffix);

            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut state.list_state);
}

fn draw_help(frame: &mut ratatui::Frame, area: Rect, state: &FileBrowserState) {
    let help_text = if state.filtering {
        "Type to filter | Esc: clear filter | Enter: accept filter"
    } else {
        "j/k/arrows: navigate  Enter: open/select  /: filter  -: parent  q/Esc: cancel"
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(
        help_text,
        Style::default().fg(Color::DarkGray),
    )))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(paragraph, area);
}
