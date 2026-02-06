use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
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
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use regex::Regex;
use std::io::{self, Stdout};

/// Result returned when the user completes both regex and format selection.
pub struct RegexFormatResult {
    pub pattern: String,
    pub format: String,
}

/// Guard that restores terminal state on drop.
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

// --- Text editing free functions ---

fn insert_char(text: &mut String, cursor: &mut usize, c: char) {
    text.insert(*cursor, c);
    *cursor += c.len_utf8();
}

fn delete_char(text: &mut String, cursor: &mut usize) {
    if *cursor > 0 {
        let prev = text[..*cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        text.replace_range(prev..*cursor, "");
        *cursor = prev;
    }
}

fn move_left(text: &str, cursor: &mut usize) {
    if *cursor > 0 {
        *cursor = text[..*cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }
}

fn move_right(text: &str, cursor: &mut usize) {
    if *cursor < text.len() {
        *cursor = text[*cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| *cursor + i)
            .unwrap_or(text.len());
    }
}

// --- Timestamp parse preview ---

fn try_parse_timestamp(value: &str, format: &str) -> Result<DateTime<Utc>, String> {
    match format {
        "iso8601" => DateTime::parse_from_rfc3339(value)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| e.to_string()),
        "epoch" => {
            let seconds: i64 = value.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
            Utc.timestamp_opt(seconds, 0)
                .single()
                .ok_or_else(|| "timestamp out of range".to_string())
        }
        "epoch_ms" => {
            let millis: i64 = value.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
            let seconds = millis / 1000;
            let nanos = ((millis % 1000) * 1_000_000) as u32;
            Utc.timestamp_opt(seconds, nanos)
                .single()
                .ok_or_else(|| "timestamp out of range".to_string())
        }
        fmt => {
            if fmt.contains("%z") || fmt.contains("%Z") || fmt.contains("%:z") {
                DateTime::parse_from_str(value, fmt)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| e.to_string())
            } else {
                NaiveDateTime::parse_from_str(value, fmt)
                    .map(|ndt| Utc.from_utc_datetime(&ndt))
                    .map_err(|e| e.to_string())
            }
        }
    }
}

// --- Phase enum ---

#[derive(Clone)]
enum Phase {
    RegexInput,
    FormatSelect { selected: usize },
    StrptimeInput,
}

const FORMAT_OPTIONS: &[(&str, &str, &str)] = &[
    ("iso8601", "ISO 8601", "e.g. 2025-12-04T02:42:11.011Z"),
    ("epoch", "Unix epoch (seconds)", "e.g. 1733280131"),
    ("epoch_ms", "Unix epoch (milliseconds)", "e.g. 1733280131011"),
    ("strptime", "Custom strptime format", "e.g. %d/%b/%Y:%H:%M:%S %z"),
];

struct AppState {
    // Phase 1: regex
    pattern: String,
    pattern_cursor: usize,
    sample_lines: Vec<String>,
    error_msg: Option<String>,
    has_ts_group: bool,
    group_names: Vec<String>,
    // Shared: captured ts value from first matching sample line
    captured_ts: Option<String>,
    // Phase 2: format select
    // (stored in Phase enum)
    // Phase 3: strptime
    strptime_fmt: String,
    strptime_cursor: usize,
    // Current phase
    phase: Phase,
}

impl AppState {
    fn new(sample_lines: &[String]) -> Self {
        Self {
            pattern: String::new(),
            pattern_cursor: 0,
            sample_lines: sample_lines.to_vec(),
            error_msg: None,
            has_ts_group: false,
            group_names: Vec::new(),
            captured_ts: None,
            strptime_fmt: String::new(),
            strptime_cursor: 0,
            phase: Phase::RegexInput,
        }
    }

    fn try_compile(&mut self) -> Option<Regex> {
        if self.pattern.is_empty() {
            self.error_msg = None;
            self.has_ts_group = false;
            self.group_names.clear();
            return None;
        }

        match Regex::new(&self.pattern) {
            Ok(re) => {
                self.group_names = re
                    .capture_names()
                    .flatten()
                    .map(|s| s.to_string())
                    .collect();
                self.has_ts_group = self.group_names.iter().any(|n| n == "ts");
                self.error_msg = None;
                Some(re)
            }
            Err(e) => {
                self.error_msg = Some(format!("{}", e));
                self.has_ts_group = false;
                self.group_names.clear();
                None
            }
        }
    }

    fn can_accept_regex(&self) -> bool {
        self.error_msg.is_none() && self.has_ts_group && !self.pattern.is_empty()
    }

    /// Extract the ts capture value from the first matching sample line.
    fn extract_ts_value(&self, re: &Regex) -> Option<String> {
        for line in &self.sample_lines {
            if let Some(caps) = re.captures(line) {
                if let Some(ts) = caps.name("ts") {
                    return Some(ts.as_str().to_string());
                }
            }
        }
        None
    }

    fn current_format_string(&self) -> Option<String> {
        match &self.phase {
            Phase::FormatSelect { selected } => {
                let (key, _, _) = FORMAT_OPTIONS[*selected];
                if key == "strptime" {
                    None // need Phase 3
                } else {
                    Some(key.to_string())
                }
            }
            Phase::StrptimeInput => {
                if self.strptime_fmt.is_empty() {
                    None
                } else {
                    Some(self.strptime_fmt.clone())
                }
            }
            _ => None,
        }
    }
}

/// Launch the regex input TUI. Returns the entered pattern, or None if cancelled.
pub fn input_regex_pattern(sample_lines: &[String]) -> io::Result<Option<String>> {
    match input_regex_and_format(sample_lines)? {
        Some(result) => Ok(Some(result.pattern)),
        None => Ok(None),
    }
}

/// Launch the combined regex + format selection TUI.
/// Returns pattern and format, or None if cancelled.
pub fn input_regex_and_format(sample_lines: &[String]) -> io::Result<Option<RegexFormatResult>> {
    let mut guard = TerminalGuard::new()?;
    let mut state = AppState::new(sample_lines);

    loop {
        let compiled = state.try_compile();

        guard.terminal.draw(|frame| {
            let area = frame.area();
            match &state.phase {
                Phase::RegexInput => draw_phase1(frame, area, &state, compiled.as_ref()),
                Phase::FormatSelect { .. } => draw_phase2(frame, area, &state, compiled.as_ref()),
                Phase::StrptimeInput => draw_phase3(frame, area, &state, compiled.as_ref()),
            }
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
            {
                return Ok(None);
            }

            match state.phase.clone() {
                Phase::RegexInput => {
                    match key.code {
                        KeyCode::Esc => return Ok(None),
                        KeyCode::Enter => {
                            if state.can_accept_regex() {
                                // Extract captured ts and transition to Phase 2
                                if let Some(re) = compiled.as_ref() {
                                    state.captured_ts = state.extract_ts_value(re);
                                }
                                state.phase = Phase::FormatSelect { selected: 0 };
                            }
                        }
                        KeyCode::Backspace => {
                            delete_char(&mut state.pattern, &mut state.pattern_cursor);
                        }
                        KeyCode::Left => {
                            move_left(&state.pattern, &mut state.pattern_cursor);
                        }
                        KeyCode::Right => {
                            move_right(&state.pattern, &mut state.pattern_cursor);
                        }
                        KeyCode::Home => state.pattern_cursor = 0,
                        KeyCode::End => state.pattern_cursor = state.pattern.len(),
                        KeyCode::Char(c) => {
                            insert_char(&mut state.pattern, &mut state.pattern_cursor, c);
                        }
                        _ => {}
                    }
                }
                Phase::FormatSelect { selected } => {
                    match key.code {
                        KeyCode::Esc => {
                            state.phase = Phase::RegexInput;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            let new_sel = if selected == 0 {
                                FORMAT_OPTIONS.len() - 1
                            } else {
                                selected - 1
                            };
                            state.phase = Phase::FormatSelect { selected: new_sel };
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let new_sel = (selected + 1) % FORMAT_OPTIONS.len();
                            state.phase = Phase::FormatSelect { selected: new_sel };
                        }
                        KeyCode::Enter => {
                            let (key, _, _) = FORMAT_OPTIONS[selected];
                            if key == "strptime" {
                                state.phase = Phase::StrptimeInput;
                            } else {
                                return Ok(Some(RegexFormatResult {
                                    pattern: state.pattern,
                                    format: key.to_string(),
                                }));
                            }
                        }
                        _ => {}
                    }
                }
                Phase::StrptimeInput => {
                    match key.code {
                        KeyCode::Esc => {
                            state.phase = Phase::FormatSelect { selected: 3 };
                        }
                        KeyCode::Enter => {
                            if !state.strptime_fmt.is_empty() {
                                return Ok(Some(RegexFormatResult {
                                    pattern: state.pattern,
                                    format: state.strptime_fmt.clone(),
                                }));
                            }
                        }
                        KeyCode::Backspace => {
                            delete_char(&mut state.strptime_fmt, &mut state.strptime_cursor);
                        }
                        KeyCode::Left => {
                            move_left(&state.strptime_fmt, &mut state.strptime_cursor);
                        }
                        KeyCode::Right => {
                            move_right(&state.strptime_fmt, &mut state.strptime_cursor);
                        }
                        KeyCode::Home => state.strptime_cursor = 0,
                        KeyCode::End => state.strptime_cursor = state.strptime_fmt.len(),
                        KeyCode::Char(c) => {
                            insert_char(&mut state.strptime_fmt, &mut state.strptime_cursor, c);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

// =============================================================================
// Phase 1: Regex Input (same UX as before)
// =============================================================================

fn draw_phase1(
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &AppState,
    compiled: Option<&Regex>,
) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // title
        Constraint::Min(3),   // sample lines
        Constraint::Length(2), // group info
        Constraint::Length(3), // input
        Constraint::Length(2), // help
    ])
    .split(area);

    draw_title(
        frame,
        chunks[0],
        " Timestamp Pattern",
        " Pattern must contain a (?P<ts>...) capture group",
    );
    draw_sample_lines(frame, chunks[1], state, compiled);
    draw_group_info(frame, chunks[2], state);
    draw_regex_input(frame, chunks[3], state);
    draw_phase1_help(frame, chunks[4], state);
}

// =============================================================================
// Phase 2: Format Select
// =============================================================================

fn draw_phase2(
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &AppState,
    compiled: Option<&Regex>,
) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // title
        Constraint::Min(3),   // sample lines (still visible)
        Constraint::Length(2), // captured ts display
        Constraint::Length(6), // format options
        Constraint::Length(2), // parse preview
        Constraint::Length(2), // help
    ])
    .split(area);

    draw_title(
        frame,
        chunks[0],
        " Timestamp Format",
        " Select the format that matches your captured timestamp",
    );
    draw_sample_lines(frame, chunks[1], state, compiled);
    draw_captured_ts(frame, chunks[2], state);
    draw_format_options(frame, chunks[3], state);
    draw_parse_preview(frame, chunks[4], state);
    draw_phase2_help(frame, chunks[5]);
}

// =============================================================================
// Phase 3: Strptime Input
// =============================================================================

fn draw_phase3(
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &AppState,
    compiled: Option<&Regex>,
) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // title
        Constraint::Length(2), // captured ts display
        Constraint::Length(3), // strptime input
        Constraint::Length(2), // parse preview
        Constraint::Length(9), // cheatsheet
        Constraint::Min(0),   // sample lines (if space)
        Constraint::Length(2), // help
    ])
    .split(area);

    draw_title(
        frame,
        chunks[0],
        " Strptime Format",
        " Enter a format string to parse the captured timestamp",
    );
    draw_captured_ts(frame, chunks[1], state);
    draw_strptime_input(frame, chunks[2], state);
    draw_parse_preview(frame, chunks[3], state);
    draw_cheatsheet(frame, chunks[4]);
    // Show sample lines in remaining space if any
    if chunks[5].height >= 2 {
        draw_sample_lines(frame, chunks[5], state, compiled);
    }
    draw_phase3_help(frame, chunks[6], state);
}

// =============================================================================
// Shared drawing helpers
// =============================================================================

fn draw_title(frame: &mut ratatui::Frame, area: Rect, title: &str, subtitle: &str) {
    let lines = vec![
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, Style::default().fg(Color::DarkGray))),
    ];

    let block = Block::default().borders(Borders::BOTTOM);
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_sample_lines(
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &AppState,
    compiled: Option<&Regex>,
) {
    let lines: Vec<Line> = state
        .sample_lines
        .iter()
        .map(|line| highlight_line(line, compiled))
        .collect();

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .title("Sample lines");
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

/// Highlight a sample line according to regex matches.
/// - Overall match: green/bold
/// - `ts` capture group: bright cyan
fn highlight_line<'a>(line: &'a str, compiled: Option<&Regex>) -> Line<'a> {
    let Some(re) = compiled else {
        return Line::from(Span::raw(line));
    };

    let Some(caps) = re.captures(line) else {
        return Line::from(Span::styled(
            line,
            Style::default().fg(Color::DarkGray),
        ));
    };

    let overall = caps.get(0).unwrap();
    let ts_match = caps.name("ts");

    let match_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let ts_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(Color::DarkGray);

    let mut spans: Vec<Span> = Vec::new();

    // Before match
    if overall.start() > 0 {
        spans.push(Span::styled(&line[..overall.start()], dim_style));
    }

    // Within the match, highlight the ts group differently
    let match_str = &line[overall.start()..overall.end()];
    if let Some(ts) = ts_match {
        let ts_start = ts.start() - overall.start();
        let ts_end = ts.end() - overall.start();

        if ts_start > 0 {
            spans.push(Span::styled(&match_str[..ts_start], match_style));
        }
        spans.push(Span::styled(&match_str[ts_start..ts_end], ts_style));
        if ts_end < match_str.len() {
            spans.push(Span::styled(&match_str[ts_end..], match_style));
        }
    } else {
        spans.push(Span::styled(match_str, match_style));
    }

    // After match
    if overall.end() < line.len() {
        spans.push(Span::styled(&line[overall.end()..], dim_style));
    }

    Line::from(spans)
}

fn draw_group_info(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let content = if !state.group_names.is_empty() {
        let groups = state.group_names.join(", ");
        let ts_status = if state.has_ts_group {
            Span::styled(" [ts: OK]", Style::default().fg(Color::Green))
        } else {
            Span::styled(
                " [ts: MISSING]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        };
        Line::from(vec![
            Span::styled(
                format!(" Groups: {}", groups),
                Style::default().fg(Color::Yellow),
            ),
            ts_status,
        ])
    } else if let Some(ref err) = state.error_msg {
        let max_len = area.width.saturating_sub(2) as usize;
        let display = if err.len() > max_len {
            &err[..max_len]
        } else {
            err
        };
        Line::from(Span::styled(
            format!(" {}", display),
            Style::default().fg(Color::Red),
        ))
    } else {
        Line::from(Span::styled(
            " Enter a regex pattern above",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let paragraph = Paragraph::new(content).block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(paragraph, area);
}

fn draw_regex_input(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let input_style = if state.error_msg.is_some() {
        Style::default().fg(Color::Red)
    } else if state.can_accept_regex() {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::White)
    };

    let display = format!(" > {}", state.pattern);
    let paragraph = Paragraph::new(Line::from(Span::styled(display, input_style)))
        .block(Block::default().borders(Borders::ALL).title("Pattern"));

    frame.render_widget(paragraph, area);

    let cursor_x = area.x + 4 + state.pattern_cursor as u16;
    let cursor_y = area.y + 1;
    if cursor_x < area.x + area.width - 1 {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn draw_phase1_help(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let help = if state.can_accept_regex() {
        "Enter: accept  Esc: cancel"
    } else if state.pattern.is_empty() {
        "Type a regex pattern with a (?P<ts>...) group  Esc: cancel"
    } else if !state.has_ts_group && state.error_msg.is_none() {
        "Pattern needs a (?P<ts>...) capture group  Esc: cancel"
    } else {
        "Fix the pattern error  Esc: cancel"
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(
        help,
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(paragraph, area);
}

// =============================================================================
// Phase 2: Format Select drawing
// =============================================================================

fn draw_captured_ts(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let content = if let Some(ref ts) = state.captured_ts {
        Line::from(vec![
            Span::styled(" Captured: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("\"{}\"", ts),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(Span::styled(
            " No timestamp captured from sample lines",
            Style::default().fg(Color::Yellow),
        ))
    };

    let paragraph = Paragraph::new(content).block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(paragraph, area);
}

fn draw_format_options(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let selected = match &state.phase {
        Phase::FormatSelect { selected } => *selected,
        _ => 0,
    };

    let lines: Vec<Line> = FORMAT_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, label, example))| {
            let marker = if i == selected { " > " } else { "   " };
            let style = if i == selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let example_style = if i == selected {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(*label, style),
                Span::styled(format!("  {}", example), example_style),
            ])
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title("Format");
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_parse_preview(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let content = match (&state.captured_ts, state.current_format_string()) {
        (Some(ts), Some(fmt)) => match try_parse_timestamp(ts, &fmt) {
            Ok(dt) => Line::from(Span::styled(
                format!(" Parsed: {}", dt.to_rfc3339()),
                Style::default().fg(Color::Green),
            )),
            Err(e) => Line::from(vec![
                Span::styled(" Does not parse: ", Style::default().fg(Color::Red)),
                Span::styled(e, Style::default().fg(Color::Yellow)),
            ]),
        },
        (None, _) => Line::from(Span::styled(
            " No captured timestamp to preview",
            Style::default().fg(Color::DarkGray),
        )),
        (_, None) => Line::from(Span::styled(
            " Enter a format string to preview",
            Style::default().fg(Color::DarkGray),
        )),
    };

    let paragraph = Paragraph::new(content);
    frame.render_widget(paragraph, area);
}

fn draw_phase2_help(frame: &mut ratatui::Frame, area: Rect) {
    let paragraph = Paragraph::new(Line::from(Span::styled(
        "Up/Down: navigate  Enter: accept  Esc: back to pattern",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(paragraph, area);
}

// =============================================================================
// Phase 3: Strptime Input drawing
// =============================================================================

fn draw_strptime_input(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let input_style = if state.strptime_fmt.is_empty() {
        Style::default().fg(Color::White)
    } else if let Some(ref ts) = state.captured_ts {
        if try_parse_timestamp(ts, &state.strptime_fmt).is_ok() {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        }
    } else {
        Style::default().fg(Color::White)
    };

    let display = format!(" > {}", state.strptime_fmt);
    let paragraph = Paragraph::new(Line::from(Span::styled(display, input_style)))
        .block(Block::default().borders(Borders::ALL).title("Format string"));

    frame.render_widget(paragraph, area);

    let cursor_x = area.x + 4 + state.strptime_cursor as u16;
    let cursor_y = area.y + 1;
    if cursor_x < area.x + area.width - 1 {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn draw_cheatsheet(frame: &mut ratatui::Frame, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::Yellow);

    let rows: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(" %Y ", val),
            Span::styled("Year (2025)      ", dim),
            Span::styled(" %m ", val),
            Span::styled("Month (01-12)    ", dim),
            Span::styled(" %d ", val),
            Span::styled("Day (01-31)", dim),
        ]),
        Line::from(vec![
            Span::styled(" %H ", val),
            Span::styled("Hour (00-23)     ", dim),
            Span::styled(" %M ", val),
            Span::styled("Minute (00-59)   ", dim),
            Span::styled(" %S ", val),
            Span::styled("Second (00-59)", dim),
        ]),
        Line::from(vec![
            Span::styled(" %b ", val),
            Span::styled("Mon abbr (Dec)   ", dim),
            Span::styled(" %B ", val),
            Span::styled("Month name       ", dim),
            Span::styled(" %a ", val),
            Span::styled("Day abbr (Mon)", dim),
        ]),
        Line::from(vec![
            Span::styled(" %z ", val),
            Span::styled("TZ +0000         ", dim),
            Span::styled(" %:z", val),
            Span::styled(" TZ +00:00       ", dim),
            Span::styled(" %Z ", val),
            Span::styled("TZ name (UTC)", dim),
        ]),
        Line::from(vec![
            Span::styled(" %3f", val),
            Span::styled(" Milliseconds    ", dim),
            Span::styled(" %f ", val),
            Span::styled("Microseconds     ", dim),
            Span::styled(" %p ", val),
            Span::styled("AM/PM", dim),
        ]),
        Line::from(vec![
            Span::styled(" %I ", val),
            Span::styled("Hour (01-12)     ", dim),
            Span::styled(" %e ", val),
            Span::styled("Day ( 1-31)      ", dim),
            Span::styled(" %% ", val),
            Span::styled("Literal %", dim),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Strptime cheatsheet");
    let paragraph = Paragraph::new(rows).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_phase3_help(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let help = if state.strptime_fmt.is_empty() {
        "Type a format string  Esc: back to format select"
    } else {
        "Enter: accept  Esc: back to format select"
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(
        help,
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(paragraph, area);
}
