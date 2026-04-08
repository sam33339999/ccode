use ccode_application::commands::agent_run::AgentRunCommand;
use ccode_bootstrap::AppState as BootstrapState;
use ccode_bootstrap::exports::ImageSource;
use ccode_bootstrap::worker_monitor;
use chrono::{DateTime, Local};
use clap::Args;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::output::{
    ErrorCategory, ErrorContext, ToolConfirmationDecision, classify_error, classify_tool_risk,
    error_category_label, render_error_message, summarize_tool_args, worker_status_label,
};

// ── Color support detection ───────────────────────────────────────────────────

/// Detected terminal color capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorSupport {
    /// No color output: `NO_COLOR` env var set or `TERM=dumb`.
    None,
    /// Standard 16-color ANSI support.
    Basic16,
}

fn detect_color_support() -> ColorSupport {
    if std::env::var_os("NO_COLOR").is_some() {
        return ColorSupport::None;
    }
    match std::env::var("TERM").as_deref() {
        Ok("dumb") | Ok("") => ColorSupport::None,
        _ => ColorSupport::Basic16,
    }
}

// ── Theme ─────────────────────────────────────────────────────────────────────

/// Named theme variants configurable via `~/.ccode/config.toml` `[tui]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeName {
    Default,
    HighContrast,
    NoColor,
}

impl ThemeName {
    fn from_str(s: &str) -> Self {
        match s {
            "high_contrast" => Self::HighContrast,
            "no_color" => Self::NoColor,
            _ => Self::Default,
        }
    }
}

/// All `Style`s used by the TUI in one place.  Constructed once at startup from
/// the detected color support and the user's configured theme name.
#[derive(Debug, Clone)]
struct Theme {
    panel_title: Style,
    user_line: Style,
    assistant_line: Style,
    worker_selected: Style,
    worker_running: Style,
    worker_completed: Style,
    worker_failed: Style,
    status_info: Style,
    status_error: Style,
    hint_line: Style,
}

impl Theme {
    fn build(theme_name: ThemeName, color_support: ColorSupport) -> Self {
        if color_support == ColorSupport::None || theme_name == ThemeName::NoColor {
            return Self::no_color();
        }
        match theme_name {
            ThemeName::HighContrast => Self::high_contrast(),
            _ => Self::default_colors(),
        }
    }

    fn default_colors() -> Self {
        Self {
            panel_title: Style::default().add_modifier(Modifier::BOLD),
            user_line: Style::default().fg(Color::Cyan),
            assistant_line: Style::default().fg(Color::Green),
            worker_selected: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            worker_running: Style::default().fg(Color::Yellow),
            worker_completed: Style::default().fg(Color::Green),
            worker_failed: Style::default().fg(Color::Red),
            status_info: Style::default().fg(Color::Cyan),
            status_error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            hint_line: Style::default().fg(Color::DarkGray),
        }
    }

    fn high_contrast() -> Self {
        Self {
            panel_title: Style::default()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            user_line: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            assistant_line: Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
            worker_selected: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            worker_running: Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            worker_completed: Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
            worker_failed: Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
            status_info: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            status_error: Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            hint_line: Style::default().fg(Color::White),
        }
    }

    fn no_color() -> Self {
        Self {
            panel_title: Style::default().add_modifier(Modifier::BOLD),
            user_line: Style::default().add_modifier(Modifier::BOLD),
            assistant_line: Style::default(),
            worker_selected: Style::default().add_modifier(Modifier::BOLD),
            worker_running: Style::default(),
            worker_completed: Style::default(),
            worker_failed: Style::default().add_modifier(Modifier::BOLD),
            status_info: Style::default(),
            status_error: Style::default().add_modifier(Modifier::BOLD),
            hint_line: Style::default(),
        }
    }
}

// ── Terminal capability check ─────────────────────────────────────────────────

/// Returns `true` when the current terminal can support full raw-mode TUI.
/// Checks for `TERM=dumb` and attempts to probe raw-mode availability.
fn terminal_supports_tui() -> bool {
    match std::env::var("TERM").as_deref() {
        Ok("dumb") | Ok("") => return false,
        _ => {}
    }
    // A quick non-destructive probe: if crossterm cannot enter raw mode, fall back.
    match enable_raw_mode() {
        Ok(()) => {
            let _ = disable_raw_mode();
            true
        }
        Err(_) => false,
    }
}

// ── Entry points ──────────────────────────────────────────────────────────────

#[derive(Args, Default)]
pub struct TuiArgs {}

pub async fn run(_args: TuiArgs) -> anyhow::Result<()> {
    run_ui().await
}

pub async fn run_ui() -> anyhow::Result<()> {
    if !terminal_supports_tui() {
        anyhow::bail!("[tui] terminal does not support raw mode");
    }
    run_ui_loop().await
}

async fn run_ui_loop() -> anyhow::Result<()> {
    // Resolve the user's preferred theme via bootstrap (best-effort; falls back to "").
    let tui_theme_name = ccode_bootstrap::tui_theme().unwrap_or_default();
    let color_support = detect_color_support();
    let theme = Theme::build(ThemeName::from_str(tui_theme_name.as_str()), color_support);

    install_panic_restoration_hook();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let terminal_guard = TerminalGuard;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let state = ccode_bootstrap::wire_from_config_with_cwd(std::env::current_dir().ok());
    let mut app = AppState::with_theme(theme);
    let (ui_tx, mut ui_rx) = tokio::sync::mpsc::unbounded_channel::<UiEvent>();
    let mut worker_monitor_rx = worker_monitor::subscribe_worker_events();
    let mut runtime = RuntimeState::default();
    let always_allowed_tools: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    let runtime_deps = match state {
        Ok(bootstrap_state) => {
            if bootstrap_state.provider.is_none() {
                app.push_error_status("no LLM provider configured".to_string());
                RuntimeDeps::Unavailable
            } else {
                RuntimeDeps::Ready {
                    bootstrap_state: Arc::new(bootstrap_state),
                }
            }
        }
        Err(e) => {
            app.push_error_status(e.to_string());
            RuntimeDeps::Unavailable
        }
    };

    let mut last_draw = Instant::now() - DrawLimiter::interval();
    let mut dirty = true;
    while !app.should_quit {
        drain_ui_events(
            &mut app,
            &mut runtime,
            &mut ui_rx,
            &mut worker_monitor_rx,
            &mut dirty,
        );

        // Auto-continue: when the coordinator's turn ended with running
        // workers and all workers have now reached a terminal state,
        // automatically send a follow-up so the coordinator can synthesize.
        if !runtime.in_flight && app.pending_worker_count > 0 && app.running_worker_count() == 0 {
            app.pending_worker_count = 0;
            if let RuntimeDeps::Ready { bootstrap_state } = &runtime_deps {
                let follow_up = app.build_worker_results_prompt();
                app.conversation
                    .push(ConversationLine::User(follow_up.clone()));
                app.push_info_status("auto-continuing: all workers finished".to_string());
                runtime.in_flight = true;
                spawn_agent_turn(
                    Arc::clone(bootstrap_state),
                    runtime.session_id.clone(),
                    follow_up,
                    Vec::new(),
                    ui_tx.clone(),
                    Arc::clone(&always_allowed_tools),
                );
                dirty = true;
            }
        }

        let timeout = DrawLimiter::next_timeout(last_draw, dirty);
        if event::poll(Duration::from_millis(50))? {
            let action = match event::read()? {
                Event::Key(key) => app.handle_input_event(AppInputEvent::Key(key)),
                Event::Paste(text) => app.handle_input_event(AppInputEvent::Paste(text)),
                _ => AppAction::None,
            };

            match action {
                AppAction::None => dirty = true,
                AppAction::Quit => app.should_quit = true,
                AppAction::Submit(prompt) => {
                    if runtime.in_flight {
                        app.push_info_status("request already in progress".to_string());
                    } else if let RuntimeDeps::Ready { bootstrap_state } = &runtime_deps {
                        match ccode_bootstrap::load_images_from_placeholders(prompt.as_str()) {
                            Ok(images) => {
                                runtime.in_flight = true;
                                spawn_agent_turn(
                                    Arc::clone(bootstrap_state),
                                    runtime.session_id.clone(),
                                    prompt,
                                    images,
                                    ui_tx.clone(),
                                    Arc::clone(&always_allowed_tools),
                                );
                            }
                            Err(err) => {
                                app.push_error_status(format!("image placeholder error: {err}"));
                            }
                        }
                    } else {
                        app.push_error_status("provider unavailable".to_string());
                    }
                    dirty = true;
                }
                AppAction::ToolApprovalResolved { name, decision } => {
                    app.push_info_status(format!("tool decision: {} {}", name, decision.label()));
                    dirty = true;
                }
                AppAction::StopSelectedTask(task_id) => {
                    if let RuntimeDeps::Ready { bootstrap_state } = &runtime_deps {
                        spawn_task_stop(Arc::clone(bootstrap_state), task_id, ui_tx.clone());
                    } else {
                        app.push_error_status("provider unavailable".to_string());
                    }
                    dirty = true;
                }
            }
        }

        if dirty && DrawLimiter::should_draw(last_draw) {
            terminal.draw(|frame| draw_ui(frame, &app))?;
            last_draw = Instant::now();
            dirty = false;
        } else if timeout > Duration::ZERO {
            std::thread::sleep(timeout.min(Duration::from_millis(10)));
        }
    }

    terminal.show_cursor()?;
    drop(terminal);
    drop(terminal_guard);
    Ok(())
}

fn draw_ui(frame: &mut Frame<'_>, app: &AppState) {
    let t = &app.theme;
    let [conversation_pane, worker_pane, status_pane, input_pane] = split_layout(frame.area());

    let conversation = Paragraph::new(app.render_conversation())
        .block(
            Block::default()
                .title("Conversation")
                .borders(Borders::ALL)
                .title_style(t.panel_title),
        )
        .scroll((app.conversation_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, conversation_pane);

    let [worker_list_pane, worker_detail_pane] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(6)]).areas(worker_pane);
    let worker_list =
        Paragraph::new(app.render_worker_list(worker_list_pane.height, worker_list_pane.width))
            .block(
                Block::default()
                    .title("Worker Tasks")
                    .borders(Borders::ALL)
                    .title_style(t.panel_title),
            )
            .wrap(Wrap { trim: false });
    frame.render_widget(worker_list, worker_list_pane);

    let worker_details = Paragraph::new(app.render_worker_details())
        .block(
            Block::default()
                .title("Worker Details")
                .borders(Borders::ALL)
                .title_style(t.panel_title),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(worker_details, worker_detail_pane);

    let status = Paragraph::new(app.render_status_lines()).block(
        Block::default()
            .title("Status")
            .borders(Borders::ALL)
            .title_style(t.panel_title),
    );
    frame.render_widget(status, status_pane);

    let input_title =
        "Input  [Enter]=send  [Shift+Enter]=newline  [Ctrl+C/Esc]=quit  [PgUp/PgDn]=scroll";
    let input = Paragraph::new(app.render_input())
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(input_title)
                .borders(Borders::ALL)
                .title_style(t.hint_line),
        );
    frame.render_widget(input, input_pane);
    if !app.has_pending_tool_approval() {
        frame.set_cursor_position(app.input_cursor_position(input_pane));
    }

    if let Some(modal) = app.tool_approval.as_ref() {
        let modal_area = centered_rect(frame.area(), 80, 14);
        frame.render_widget(Clear, modal_area);
        let modal_widget = Paragraph::new(modal.render_lines())
            .block(
                Block::default()
                    .title("Tool Approval Required  [y]=allow  [n]=deny  [a]=always-allow")
                    .borders(Borders::ALL)
                    .title_style(t.panel_title),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(modal_widget, modal_area);
    }
}

fn split_layout(area: Rect) -> [Rect; 4] {
    let [top_pane, status_pane, input_pane] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(5),
        Constraint::Length(3),
    ])
    .areas(area);
    let [conversation_pane, worker_pane] =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
            .areas(top_pane);
    [conversation_pane, worker_pane, status_pane, input_pane]
}

fn centered_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    let preferred_width = (area.width.saturating_mul(width_percent) / 100).max(40);
    let width = preferred_width.min(area.width.saturating_sub(2).max(1));
    let preferred_height = height.min(area.height.saturating_sub(2)).max(8);
    let height = preferred_height.min(area.height.saturating_sub(2).max(1));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

#[derive(Default)]
struct RuntimeState {
    in_flight: bool,
    session_id: Option<String>,
}

enum RuntimeDeps {
    Ready {
        bootstrap_state: Arc<BootstrapState>,
    },
    Unavailable,
}

#[derive(Clone, Debug)]
enum StatusKind {
    Info,
    Error(ErrorCategory),
}

#[derive(Clone, Debug)]
struct StatusLine {
    kind: StatusKind,
    message: String,
}

#[derive(Clone, Debug)]
enum ConversationLine {
    User(String),
    Assistant(String),
    WorkerStatus { task_id: String, status: String },
}

struct AppState {
    conversation: Vec<ConversationLine>,
    status: VecDeque<StatusLine>,
    input: InputBuffer,
    input_history: VecDeque<String>,
    history_cursor: Option<usize>,
    history_draft: Option<String>,
    ime_preedit: Option<String>,
    suppress_enter_submit_once: bool,
    active_assistant_idx: Option<usize>,
    conversation_scroll: u16,
    tool_approval: Option<ToolApprovalModal>,
    worker_panel: WorkerPanelState,
    /// Number of workers that were Running when the last assistant turn ended.
    pending_worker_count: usize,
    should_quit: bool,
    theme: Theme,
}

impl Default for AppState {
    fn default() -> Self {
        Self::with_theme(Theme::no_color())
    }
}

impl AppState {
    fn with_theme(theme: Theme) -> Self {
        Self {
            conversation: Vec::new(),
            status: VecDeque::new(),
            input: InputBuffer::default(),
            input_history: VecDeque::new(),
            history_cursor: None,
            history_draft: None,
            ime_preedit: None,
            suppress_enter_submit_once: false,
            active_assistant_idx: None,
            conversation_scroll: 0,
            tool_approval: None,
            worker_panel: WorkerPanelState::default(),
            pending_worker_count: 0,
            should_quit: false,
            theme,
        }
    }
}

#[derive(Debug)]
enum AppAction {
    None,
    Submit(String),
    ToolApprovalResolved {
        name: String,
        decision: ToolApprovalAction,
    },
    StopSelectedTask(String),
    Quit,
}

#[derive(Clone, Debug)]
enum AppInputEvent {
    Key(KeyEvent),
    Paste(String),
}

#[derive(Default)]
struct InputBuffer {
    text: String,
    cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolApprovalAction {
    AllowOnce,
    Deny,
    AllowAlways,
}

impl ToolApprovalAction {
    fn from_selection(selected: usize) -> Self {
        match selected {
            1 => Self::Deny,
            2 => Self::AllowAlways,
            _ => Self::AllowOnce,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow",
            Self::Deny => "deny",
            Self::AllowAlways => "always-allow",
        }
    }

    fn to_confirmation_decision(self) -> ToolConfirmationDecision {
        match self {
            Self::AllowOnce => ToolConfirmationDecision::AllowOnce,
            Self::Deny => ToolConfirmationDecision::Deny,
            Self::AllowAlways => ToolConfirmationDecision::AllowAlways,
        }
    }
}

struct ToolApprovalModal {
    tool_name: String,
    params_summary: String,
    risk_level: String,
    selected: usize,
    response_tx: Option<tokio::sync::oneshot::Sender<ToolConfirmationDecision>>,
}

impl ToolApprovalModal {
    fn render_lines(&self) -> Vec<Line<'static>> {
        let actions = [
            ("Allow once [Y]", 0usize),
            ("Deny [N]", 1usize),
            ("Always allow [A]", 2usize),
        ];
        let action_line = actions
            .iter()
            .map(|(label, idx)| {
                if self.selected == *idx {
                    format!("> {label} <")
                } else {
                    (*label).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("   ");

        vec![
            Line::from(format!("Tool: {}", self.tool_name)),
            Line::from(format!("Parameters: {}", self.params_summary)),
            Line::from(format!("Risk: {}", self.risk_level)),
            Line::from(""),
            Line::from(format!("Actions: {action_line}")),
            Line::from("Keys: y=allow n=deny a=always-allow"),
            Line::from("Navigate: Tab/Shift+Tab or Left/Right, Enter=confirm, Esc=deny"),
        ]
    }
}

impl InputBuffer {
    fn as_str(&self) -> &str {
        self.text.as_str()
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn set_text(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
    }

    fn insert_char(&mut self, c: char) {
        let mut buf = [0; 4];
        self.insert_str(c.encode_utf8(&mut buf));
    }

    fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn move_left(&mut self) {
        self.cursor = prev_grapheme_boundary(self.text.as_str(), self.cursor);
    }

    fn move_right(&mut self) {
        self.cursor = next_grapheme_boundary(self.text.as_str(), self.cursor);
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.text.len();
    }

    fn backspace_grapheme(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = prev_grapheme_boundary(self.text.as_str(), self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    fn cursor_display_offset(&self) -> (u16, u16) {
        let before = &self.text[..self.cursor];
        let mut row = 0usize;
        let mut col = 0usize;
        for (idx, line) in before.split('\n').enumerate() {
            if idx > 0 {
                row += 1;
            }
            col = UnicodeWidthStr::width(line);
        }
        (row as u16, col as u16)
    }
}

impl AppState {
    fn handle_input_event(&mut self, event: AppInputEvent) -> AppAction {
        match event {
            AppInputEvent::Key(key) => self.handle_key(key),
            AppInputEvent::Paste(text) => {
                self.clear_preedit_state();
                self.input.insert_str(text.as_str());
                self.clear_history_navigation();
                AppAction::None
            }
        }
    }

    #[cfg(test)]
    fn ime_preedit_for_test(&mut self, text: String) {
        self.ime_preedit = if text.is_empty() { None } else { Some(text) };
    }

    #[cfg(test)]
    fn ime_commit_for_test(&mut self, text: String) {
        if !text.is_empty() {
            self.input.insert_str(text.as_str());
            self.suppress_enter_submit_once = true;
        }
        self.ime_preedit = None;
        self.clear_history_navigation();
    }

    fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.kind == KeyEventKind::Release {
            return AppAction::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return AppAction::Quit;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v') {
            return self.handle_ctrl_v_with(|| {
                ccode_bootstrap::paste_image_from_clipboard_to_temp_file()
                    .map_err(|e| e.to_string())
            });
        }
        if self.tool_approval.is_some() {
            return self.handle_approval_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.ime_preedit = None;
                AppAction::Quit
            }
            KeyCode::Char('q') if self.input.is_empty() => AppAction::Quit,
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.clear_preedit_state();
                self.input.insert_char('\n');
                self.clear_history_navigation();
                AppAction::None
            }
            KeyCode::Enter if self.ime_preedit.is_some() || self.suppress_enter_submit_once => {
                self.suppress_enter_submit_once = false;
                AppAction::None
            }
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => {
                self.clear_preedit_state();
                self.input.backspace_grapheme();
                self.clear_history_navigation();
                AppAction::None
            }
            KeyCode::Left => {
                self.input.move_left();
                AppAction::None
            }
            KeyCode::Right => {
                self.input.move_right();
                AppAction::None
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
                self.select_prev_worker_task();
                AppAction::None
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
                self.select_next_worker_task();
                AppAction::None
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.stop_selected_worker_task_action()
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => self.scroll_up(1),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => self.scroll_down(1),
            KeyCode::Up => {
                self.recall_history_prev();
                AppAction::None
            }
            KeyCode::Down => {
                self.recall_history_next();
                AppAction::None
            }
            KeyCode::PageUp => self.scroll_up(10),
            KeyCode::PageDown => self.scroll_down(10),
            KeyCode::Home => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.conversation_scroll = 0;
                } else {
                    self.input.move_to_start();
                }
                AppAction::None
            }
            KeyCode::End => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.conversation_scroll = u16::MAX;
                } else {
                    self.input.move_to_end();
                }
                AppAction::None
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.clear_preedit_state();
                self.input.insert_char(c);
                self.clear_history_navigation();
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn submit_input(&mut self) -> AppAction {
        let raw_input = self.input.as_str().to_string();
        let normalized = raw_input.trim().to_string();
        if normalized.is_empty() {
            return AppAction::None;
        }
        if matches!(normalized.as_str(), "q" | "quit" | "exit") {
            return AppAction::Quit;
        }
        self.push_history(raw_input.clone());

        self.conversation
            .push(ConversationLine::User(raw_input.clone()));
        self.active_assistant_idx = None;
        self.input.clear();
        self.clear_history_navigation();
        self.ime_preedit = None;
        AppAction::Submit(raw_input)
    }

    fn handle_ctrl_v_with<F>(&mut self, paste_fn: F) -> AppAction
    where
        F: FnOnce() -> Result<std::path::PathBuf, String>,
    {
        self.clear_preedit_state();
        match paste_fn() {
            Ok(path) => {
                let placeholder = format!("@image:{} ", path.display());
                self.input.insert_str(placeholder.as_str());
                self.clear_history_navigation();
                self.push_info_status(format!("pasted image: {}", path.display()));
            }
            Err(err) => {
                self.push_error_status(format!("clipboard paste failed: {err}"));
            }
        }
        AppAction::None
    }

    fn clear_preedit_state(&mut self) {
        self.ime_preedit = None;
        self.suppress_enter_submit_once = false;
    }

    fn clear_history_navigation(&mut self) {
        self.history_cursor = None;
        self.history_draft = None;
    }

    fn push_history(&mut self, item: String) {
        const MAX_HISTORY: usize = 100;
        if self
            .input_history
            .back()
            .is_some_and(|last| last.as_str() == item.as_str())
        {
            return;
        }
        if self.input_history.len() >= MAX_HISTORY {
            self.input_history.pop_front();
        }
        self.input_history.push_back(item);
    }

    fn recall_history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }

        let next_idx = match self.history_cursor {
            Some(current) => current.saturating_sub(1),
            None => {
                self.history_draft = Some(self.input.as_str().to_string());
                self.input_history.len() - 1
            }
        };
        self.history_cursor = Some(next_idx);
        if let Some(entry) = self.input_history.get(next_idx) {
            self.input.set_text(entry.clone());
        }
        self.clear_preedit_state();
    }

    fn recall_history_next(&mut self) {
        let Some(current) = self.history_cursor else {
            return;
        };

        if current + 1 < self.input_history.len() {
            let next_idx = current + 1;
            self.history_cursor = Some(next_idx);
            if let Some(entry) = self.input_history.get(next_idx) {
                self.input.set_text(entry.clone());
            }
        } else {
            self.history_cursor = None;
            let draft = self.history_draft.take().unwrap_or_default();
            self.input.set_text(draft);
        }
        self.clear_preedit_state();
    }

    fn apply_delta(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        let idx = if let Some(idx) = self.active_assistant_idx {
            idx
        } else {
            self.conversation
                .push(ConversationLine::Assistant(String::new()));
            let idx = self.conversation.len() - 1;
            self.active_assistant_idx = Some(idx);
            idx
        };
        if let Some(ConversationLine::Assistant(content)) = self.conversation.get_mut(idx) {
            content.push_str(delta);
        }
    }

    fn close_active_assistant(&mut self) {
        self.active_assistant_idx = None;
        // Snapshot how many workers are still running when the turn ends.
        self.pending_worker_count = self.running_worker_count();
    }

    fn running_worker_count(&self) -> usize {
        self.worker_panel
            .tasks
            .iter()
            .filter(|t| t.status == "Running")
            .count()
    }

    /// Build a follow-up prompt summarising completed workers so the
    /// coordinator can synthesize results and continue.
    fn build_worker_results_prompt(&self) -> String {
        let mut parts = Vec::new();
        for task in &self.worker_panel.tasks {
            if task.status == "Running" {
                continue;
            }
            let summary = task.summary.as_deref().unwrap_or("(no summary)");
            parts.push(format!("- {} [{}]: {}", task.task_id, task.status, summary));
        }
        format!(
            "All background workers have finished. Here are the results:\n{}\n\n\
             IMPORTANT: To continue a conversation with an existing worker, \
             pass its session_id (shown in brackets above as [session_id=...]) \
             to the agent tool. Do NOT create new agents for tasks that already \
             have a session — resume the existing session instead.\n\
             Synthesize the results and continue.",
            parts.join("\n")
        )
    }

    fn open_tool_approval(
        &mut self,
        name: String,
        args: Value,
        response_tx: tokio::sync::oneshot::Sender<ToolConfirmationDecision>,
    ) {
        self.tool_approval = Some(ToolApprovalModal {
            risk_level: classify_tool_risk(name.as_str()).label().to_string(),
            tool_name: name,
            params_summary: summarize_tool_args(&args),
            selected: 0,
            response_tx: Some(response_tx),
        });
    }

    #[cfg(test)]
    fn open_tool_approval_for_test(&mut self, name: String, args: Value) {
        self.tool_approval = Some(ToolApprovalModal {
            risk_level: classify_tool_risk(name.as_str()).label().to_string(),
            tool_name: name,
            params_summary: summarize_tool_args(&args),
            selected: 0,
            response_tx: None,
        });
    }

    fn has_pending_tool_approval(&self) -> bool {
        self.tool_approval.is_some()
    }

    fn handle_approval_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.resolve_tool_approval(ToolApprovalAction::AllowOnce)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.resolve_tool_approval(ToolApprovalAction::Deny)
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.resolve_tool_approval(ToolApprovalAction::AllowAlways)
            }
            KeyCode::Tab | KeyCode::Right => {
                if let Some(modal) = self.tool_approval.as_mut() {
                    modal.selected = (modal.selected + 1) % 3;
                }
                AppAction::None
            }
            KeyCode::BackTab | KeyCode::Left => {
                if let Some(modal) = self.tool_approval.as_mut() {
                    modal.selected = if modal.selected == 0 {
                        2
                    } else {
                        modal.selected - 1
                    };
                }
                AppAction::None
            }
            KeyCode::Enter => {
                let selected = self
                    .tool_approval
                    .as_ref()
                    .map(|modal| modal.selected)
                    .unwrap_or(0);
                self.resolve_tool_approval(ToolApprovalAction::from_selection(selected))
            }
            _ => AppAction::None,
        }
    }

    fn resolve_tool_approval(&mut self, decision: ToolApprovalAction) -> AppAction {
        let Some(mut modal) = self.tool_approval.take() else {
            return AppAction::None;
        };
        if let Some(response_tx) = modal.response_tx.take() {
            let _ = response_tx.send(decision.to_confirmation_decision());
        }
        AppAction::ToolApprovalResolved {
            name: modal.tool_name,
            decision,
        }
    }

    fn push_tool_start(&mut self, name: String, args: Value) {
        self.push_info_status(format!(
            "[tool:start] {name} ({})",
            summarize_tool_args(&args)
        ));
    }

    fn push_tool_done(&mut self, name: String, success: bool) {
        let marker = if success { "[ok]" } else { "[fail]" };
        self.push_info_status(format!("[tool:done] {name} {marker}"));
    }

    fn push_tool_result(&mut self, name: String, output: String) {
        if output.is_empty() {
            self.push_info_status(format!("[tool:result] {name}"));
            return;
        }
        let first_line = output.lines().next().unwrap_or_default();
        self.push_info_status(format!("[tool:result] {name} {first_line}"));
    }

    fn push_worker_status_with_details(
        &mut self,
        task_id: String,
        status: String,
        summary: Option<String>,
        timestamp: SystemTime,
    ) {
        self.conversation.push(ConversationLine::WorkerStatus {
            task_id: task_id.clone(),
            status: status.clone(),
        });
        self.record_worker_event(WorkerUiEvent {
            task_id,
            status,
            summary,
            timestamp,
        });
    }

    fn push_error_status(&mut self, message: String) {
        let category = classify_error(&message);
        self.push_status(StatusLine {
            kind: StatusKind::Error(category),
            message,
        });
    }

    fn push_info_status(&mut self, message: String) {
        self.push_status(StatusLine {
            kind: StatusKind::Info,
            message,
        });
    }

    fn push_status(&mut self, line: StatusLine) {
        const MAX_STATUS: usize = 30;
        if self.status.len() >= MAX_STATUS {
            self.status.pop_front();
        }
        self.status.push_back(line);
    }

    fn scroll_up(&mut self, amount: u16) -> AppAction {
        self.conversation_scroll = self.conversation_scroll.saturating_sub(amount);
        AppAction::None
    }

    fn scroll_down(&mut self, amount: u16) -> AppAction {
        self.conversation_scroll = self.conversation_scroll.saturating_add(amount);
        AppAction::None
    }

    fn render_conversation(&self) -> Vec<Line<'static>> {
        let t = &self.theme;
        self.conversation
            .iter()
            .flat_map(|entry| match entry {
                ConversationLine::User(text) => {
                    vec![Line::from(vec![
                        Span::styled("You: ", t.user_line),
                        Span::raw(text.clone()),
                    ])]
                }
                ConversationLine::Assistant(text) => {
                    if text.is_empty() {
                        vec![Line::from(vec![Span::styled(
                            "Assistant:",
                            t.assistant_line,
                        )])]
                    } else {
                        let mut lines = Vec::new();
                        for (idx, line) in text.lines().enumerate() {
                            if idx == 0 {
                                lines.push(Line::from(vec![
                                    Span::styled("Assistant: ", t.assistant_line),
                                    Span::raw(line.to_string()),
                                ]));
                            } else {
                                lines.push(Line::from(format!("  {line}")));
                            }
                        }
                        if text.ends_with('\n') {
                            lines.push(Line::from("  "));
                        }
                        lines
                    }
                }
                ConversationLine::WorkerStatus { task_id, status } => {
                    vec![Line::from(format!("[worker] {task_id} {status}"))]
                }
            })
            .collect()
    }

    fn render_status_lines(&self) -> Vec<Line<'static>> {
        self.status
            .iter()
            .rev()
            .take(4)
            .map(|line| match &line.kind {
                StatusKind::Info => Line::from(vec![
                    Span::styled("[*] ", self.theme.status_info),
                    Span::styled(format!("[info] {}", line.message), self.theme.status_info),
                ]),
                StatusKind::Error(category) => Line::from(vec![
                    Span::styled("[!] ", self.theme.status_error),
                    Span::styled(
                        format!("[{}] {}", error_category_label(*category), line.message),
                        self.theme.status_error,
                    ),
                ]),
            })
            .collect()
    }

    fn render_input(&self) -> String {
        let Some(preedit) = &self.ime_preedit else {
            return self.input.as_str().to_string();
        };
        if preedit.is_empty() {
            return self.input.as_str().to_string();
        }

        let cursor = self.input.cursor;
        let mut rendered = self.input.as_str().to_string();
        rendered.insert_str(cursor, preedit.as_str());
        rendered
    }

    fn record_worker_event(&mut self, event: WorkerUiEvent) {
        const MAX_WORKER_TASKS: usize = 500;
        let selected_id = if self.worker_panel.manual_selection {
            self.worker_panel
                .tasks
                .get(self.worker_panel.selected)
                .map(|task| task.task_id.clone())
        } else {
            None
        };

        if let Some(existing_idx) = self
            .worker_panel
            .tasks
            .iter()
            .position(|task| task.task_id == event.task_id)
        {
            let mut existing = self.worker_panel.tasks.remove(existing_idx);
            existing.status = event.status;
            if event.summary.is_some() {
                existing.summary = event.summary;
            }
            existing.updated_at = event.timestamp;
            self.worker_panel.tasks.insert(0, existing);
        } else {
            self.worker_panel.tasks.insert(
                0,
                WorkerTaskEntry {
                    task_id: event.task_id,
                    status: event.status,
                    summary: event.summary,
                    started_at: event.timestamp,
                    updated_at: event.timestamp,
                },
            );
        }

        if self.worker_panel.tasks.len() > MAX_WORKER_TASKS {
            self.worker_panel.tasks.truncate(MAX_WORKER_TASKS);
        }

        if let Some(selected_id) = selected_id {
            if let Some(idx) = self
                .worker_panel
                .tasks
                .iter()
                .position(|task| task.task_id == selected_id)
            {
                self.worker_panel.selected = idx;
            }
        } else {
            self.worker_panel.selected = 0;
        }
        if self.worker_panel.selected >= self.worker_panel.tasks.len() {
            self.worker_panel.selected = self.worker_panel.tasks.len().saturating_sub(1);
        }
    }

    fn selected_worker_task(&self) -> Option<&WorkerTaskEntry> {
        self.worker_panel.tasks.get(self.worker_panel.selected)
    }

    fn select_next_worker_task(&mut self) {
        if self.worker_panel.tasks.is_empty() {
            return;
        }
        self.worker_panel.manual_selection = true;
        self.worker_panel.selected =
            (self.worker_panel.selected + 1).min(self.worker_panel.tasks.len() - 1);
    }

    fn select_prev_worker_task(&mut self) {
        if self.worker_panel.tasks.is_empty() {
            return;
        }
        self.worker_panel.manual_selection = true;
        self.worker_panel.selected = self.worker_panel.selected.saturating_sub(1);
    }

    fn stop_selected_worker_task_action(&self) -> AppAction {
        let Some(selected) = self.selected_worker_task() else {
            return AppAction::None;
        };
        if selected.status == "Running" {
            AppAction::StopSelectedTask(selected.task_id.clone())
        } else {
            AppAction::None
        }
    }

    fn render_worker_list(&self, height: u16, width: u16) -> Vec<Line<'static>> {
        if self.worker_panel.tasks.is_empty() {
            return vec![
                Line::from(vec![Span::styled(
                    "No worker tasks yet",
                    self.theme.hint_line,
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Keys: Alt+Up/Alt+Down=select  Alt+S=stop",
                    self.theme.hint_line,
                )]),
            ];
        }

        let available_rows = height.saturating_sub(2).max(1) as usize;
        let start = self
            .worker_panel
            .viewport_start
            .min(self.worker_panel.tasks.len().saturating_sub(1));
        let selected = self.worker_panel.selected;
        let mut effective_start = start;
        if selected < effective_start {
            effective_start = selected;
        } else if selected >= effective_start.saturating_add(available_rows) {
            effective_start = selected.saturating_add(1).saturating_sub(available_rows);
        }
        let end = (effective_start + available_rows).min(self.worker_panel.tasks.len());

        let mut lines = Vec::new();
        for idx in effective_start..end {
            let task = &self.worker_panel.tasks[idx];
            let is_selected = idx == self.worker_panel.selected;
            let marker = if is_selected { ">" } else { " " };
            let status_style = match task.status.as_str() {
                "Running" => self.theme.worker_running,
                "Completed" => self.theme.worker_completed,
                "Failed" | "Cancelled" => self.theme.worker_failed,
                _ => Style::default(),
            };
            let row_style = if is_selected {
                self.theme.worker_selected
            } else {
                Style::default()
            };
            let task_id_truncated =
                truncate_to_width(task.task_id.as_str(), width.saturating_sub(16) as usize);
            let line = Line::from(vec![
                Span::styled(format!("{marker} "), row_style),
                Span::styled(format!("[{}]", task.status), status_style),
                Span::styled(format!(" {task_id_truncated}"), row_style),
            ]);
            lines.push(line);
        }
        lines
    }

    fn render_worker_details(&self) -> Vec<Line<'static>> {
        let Some(task) = self.selected_worker_task() else {
            return vec![
                Line::from("task_id: -"),
                Line::from("summary: -"),
                Line::from("started_at: -"),
                Line::from("updated_at: -"),
                Line::from(""),
                Line::from("Alt+S stops selected Running task"),
            ];
        };

        vec![
            Line::from(format!("task_id: {}", task.task_id)),
            Line::from(format!(
                "summary: {}",
                task.summary.clone().unwrap_or_else(|| "-".to_string())
            )),
            Line::from(format!("started_at: {}", format_task_time(task.started_at))),
            Line::from(format!("updated_at: {}", format_task_time(task.updated_at))),
            Line::from(""),
            Line::from("Alt+S stops selected Running task"),
        ]
    }

    fn input_cursor_position(&self, input_pane: Rect) -> (u16, u16) {
        let inner_x = input_pane.x.saturating_add(1);
        let inner_y = input_pane.y.saturating_add(1);
        let inner_width = input_pane.width.saturating_sub(2).max(1);
        let inner_height = input_pane.height.saturating_sub(2).max(1);

        let (mut row, mut col) = self.input.cursor_display_offset();
        row = row.saturating_add(col / inner_width);
        col %= inner_width;

        (
            inner_x.saturating_add(col.min(inner_width.saturating_sub(1))),
            inner_y.saturating_add(row.min(inner_height.saturating_sub(1))),
        )
    }
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        if UnicodeWidthStr::width(candidate.as_str()) > max_width {
            break;
        }
        out.push(ch);
    }
    out
}

fn format_task_time(ts: SystemTime) -> String {
    let dt: DateTime<Local> = DateTime::from(ts);
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn prev_grapheme_boundary(text: &str, cursor: usize) -> usize {
    let mut prev = 0usize;
    for (idx, _) in text.grapheme_indices(true) {
        if idx >= cursor {
            break;
        }
        prev = idx;
    }
    prev
}

fn next_grapheme_boundary(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    for (idx, _) in text.grapheme_indices(true) {
        if idx > cursor {
            return idx;
        }
    }
    text.len()
}

enum UiEvent {
    AssistantDelta(String),
    AssistantDone,
    ToolStart {
        name: String,
        args: Value,
    },
    ToolDone {
        name: String,
        success: bool,
    },
    ToolResult {
        name: String,
        output: String,
    },
    ToolApprovalRequested {
        name: String,
        args: Value,
        response_tx: tokio::sync::oneshot::Sender<ToolConfirmationDecision>,
    },
    ToolError {
        name: String,
        message: String,
    },
    WorkerStatus {
        task_id: String,
        status: String,
        summary: Option<String>,
        timestamp: SystemTime,
    },
    Error(String),
    SessionReady(String),
}

#[derive(Clone, Debug)]
struct WorkerUiEvent {
    task_id: String,
    status: String,
    summary: Option<String>,
    timestamp: SystemTime,
}

#[derive(Clone, Debug)]
struct WorkerTaskEntry {
    task_id: String,
    status: String,
    summary: Option<String>,
    started_at: SystemTime,
    updated_at: SystemTime,
}

#[derive(Default)]
struct WorkerPanelState {
    tasks: Vec<WorkerTaskEntry>,
    selected: usize,
    viewport_start: usize,
    manual_selection: bool,
}

#[derive(Clone, Copy)]
struct DrawLimiter;

impl DrawLimiter {
    const fn interval() -> Duration {
        Duration::from_millis(16)
    }

    fn should_draw(last_draw: Instant) -> bool {
        last_draw.elapsed() >= Self::interval()
    }

    fn next_timeout(last_draw: Instant, dirty: bool) -> Duration {
        if !dirty {
            return Duration::from_millis(10);
        }
        let elapsed = last_draw.elapsed();
        if elapsed >= Self::interval() {
            Duration::ZERO
        } else {
            Self::interval() - elapsed
        }
    }
}

fn drain_ui_events(
    app: &mut AppState,
    runtime: &mut RuntimeState,
    ui_rx: &mut tokio::sync::mpsc::UnboundedReceiver<UiEvent>,
    worker_monitor_rx: &mut tokio::sync::broadcast::Receiver<
        Arc<worker_monitor::WorkerMonitorEvent>,
    >,
    dirty: &mut bool,
) {
    while let Ok(evt) = ui_rx.try_recv() {
        match evt {
            UiEvent::AssistantDelta(delta) => app.apply_delta(&delta),
            UiEvent::AssistantDone => {
                runtime.in_flight = false;
                app.close_active_assistant();
            }
            UiEvent::ToolStart { name, args } => app.push_tool_start(name, args),
            UiEvent::ToolDone { name, success } => app.push_tool_done(name, success),
            UiEvent::ToolResult { name, output } => app.push_tool_result(name, output),
            UiEvent::ToolApprovalRequested {
                name,
                args,
                response_tx,
            } => app.open_tool_approval(name, args, response_tx),
            UiEvent::ToolError { name, message } => {
                app.push_error_status(format!("tool {name}: {message}"));
            }
            UiEvent::WorkerStatus {
                task_id,
                status,
                summary,
                timestamp,
            } => app.push_worker_status_with_details(task_id, status, summary, timestamp),
            UiEvent::Error(message) => {
                runtime.in_flight = false;
                app.push_error_status(message);
            }
            UiEvent::SessionReady(sid) => {
                runtime.session_id = Some(sid.clone());
                app.push_info_status(format!("session: {sid}"));
            }
        }
        *dirty = true;
    }

    while let Ok(evt) = worker_monitor_rx.try_recv() {
        app.push_worker_status_with_details(
            evt.task_id.clone(),
            evt.status.clone(),
            evt.summary.clone(),
            evt.timestamp,
        );
        *dirty = true;
    }
}

fn spawn_agent_turn(
    bootstrap_state: Arc<BootstrapState>,
    session_id: Option<String>,
    user_content: String,
    images: Vec<ImageSource>,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
    always_allowed_tools: Arc<Mutex<HashSet<String>>>,
) {
    tokio::spawn(async move {
        let Some(provider) = bootstrap_state.provider.clone() else {
            let _ = ui_tx.send(UiEvent::Error("provider unavailable".to_string()));
            return;
        };
        let provider_name = provider.name().to_string();
        let session_for_errors = session_id.clone().unwrap_or_else(|| "new".to_string());
        let cmd = AgentRunCommand::new(Arc::clone(&bootstrap_state.session_repo), provider)
            .with_context(bootstrap_state.context_policy.clone());
        let tool_registry = Arc::clone(&bootstrap_state.tool_registry);
        let tool_ctx = Arc::new(bootstrap_state.tool_ctx());
        let tool_definitions = tool_registry.definitions();
        let tool_event_tx = ui_tx.clone();

        let on_delta = {
            let tx = ui_tx.clone();
            move |content: String| {
                let _ = tx.send(UiEvent::AssistantDelta(content));
            }
        };

        let execute_tool = move |name: String,
                                 args: Value|
              -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
        > {
            let registry = Arc::clone(&tool_registry);
            let tool_ctx = Arc::clone(&tool_ctx);
            let tx = tool_event_tx.clone();
            let always_allowed_tools = Arc::clone(&always_allowed_tools);
            Box::pin(async move {
                let _ = tx.send(UiEvent::ToolStart {
                    name: name.clone(),
                    args: args.clone(),
                });
                let is_always_allowed = always_allowed_tools
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .contains(&name);
                if !is_always_allowed {
                    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                    if tx
                        .send(UiEvent::ToolApprovalRequested {
                            name: name.clone(),
                            args: args.clone(),
                            response_tx,
                        })
                        .is_err()
                    {
                        return Err("approval prompt unavailable".to_string());
                    }
                    let decision = response_rx.await.unwrap_or(ToolConfirmationDecision::Deny);
                    match decision {
                        ToolConfirmationDecision::Deny => return Err("user denied".to_string()),
                        ToolConfirmationDecision::AllowAlways => {
                            always_allowed_tools
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .insert(name.clone());
                        }
                        ToolConfirmationDecision::AllowOnce => {}
                    }
                }
                let result = registry
                    .execute(&name, args, &tool_ctx)
                    .await
                    .map_err(|e| e.to_string());
                if let Ok(payload) = &result
                    && let Ok(value) = serde_json::from_str::<Value>(payload)
                    && let (Some(task_id), Some(status_raw)) = (
                        value.get("task_id").and_then(Value::as_str),
                        value.get("status").and_then(Value::as_str),
                    )
                    && let Some(status) = worker_status_label(status_raw)
                {
                    let summary = value
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let _ = tx.send(UiEvent::WorkerStatus {
                        task_id: task_id.to_string(),
                        status: status.to_string(),
                        summary,
                        timestamp: SystemTime::now(),
                    });
                }
                // Show sub-agent responses in the conversation pane.
                if let Ok(payload) = &result
                    && let Ok(value) = serde_json::from_str::<Value>(payload)
                    && let Some(response) = value.get("response").and_then(Value::as_str)
                    && !response.is_empty()
                {
                    let _ = tx.send(UiEvent::ToolResult {
                        name: name.clone(),
                        output: response.to_string(),
                    });
                }
                let _ = tx.send(UiEvent::ToolDone {
                    name: name.clone(),
                    success: result.is_ok(),
                });
                if let Err(err) = &result {
                    let _ = tx.send(UiEvent::ToolError {
                        name: name.clone(),
                        message: err.clone(),
                    });
                }
                result
            })
        };

        let result = cmd
            .run_with_metrics(
                session_id,
                None,
                user_content,
                images,
                tool_definitions,
                &on_delta,
                &execute_tool,
            )
            .await;

        match result {
            Ok(outcome) => {
                let _ = ui_tx.send(UiEvent::AssistantDone);
                let _ = ui_tx.send(UiEvent::SessionReady(outcome.session_id.to_string()));
            }
            Err(err) => {
                let _ = ui_tx.send(UiEvent::AssistantDone);
                let rendered = render_error_message(
                    &err.to_string(),
                    &ErrorContext {
                        session_id: session_for_errors,
                        provider_name,
                    },
                );
                let _ = ui_tx.send(UiEvent::Error(rendered));
            }
        }
    });
}

fn spawn_task_stop(
    bootstrap_state: Arc<BootstrapState>,
    task_id: String,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
) {
    tokio::spawn(async move {
        let registry = Arc::clone(&bootstrap_state.tool_registry);
        let tool_ctx = Arc::new(bootstrap_state.tool_ctx());
        let args = serde_json::json!({
            "task_id": task_id.clone(),
            "summary": "stopped from tui worker panel"
        });

        let _ = ui_tx.send(UiEvent::ToolStart {
            name: "task_stop".to_string(),
            args: args.clone(),
        });

        let result = registry
            .execute("task_stop", args, &tool_ctx)
            .await
            .map_err(|err| err.to_string());

        if let Ok(payload) = &result
            && let Ok(value) = serde_json::from_str::<Value>(payload)
            && let (Some(task_id), Some(status_raw)) = (
                value.get("task_id").and_then(Value::as_str),
                value.get("status").and_then(Value::as_str),
            )
            && let Some(status) = worker_status_label(status_raw)
        {
            let summary = value
                .get("summary")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let _ = ui_tx.send(UiEvent::WorkerStatus {
                task_id: task_id.to_string(),
                status: status.to_string(),
                summary,
                timestamp: SystemTime::now(),
            });
        }

        let _ = ui_tx.send(UiEvent::ToolDone {
            name: "task_stop".to_string(),
            success: result.is_ok(),
        });

        if let Err(message) = result {
            let _ = ui_tx.send(UiEvent::ToolError {
                name: "task_stop".to_string(),
                message,
            });
        }
    });
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn install_panic_restoration_hook() {
    static INIT_HOOK: std::sync::Once = std::sync::Once::new();

    INIT_HOOK.call_once(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = restore_terminal();
            original(panic_info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::{
        AppAction, AppState, ConversationLine, DrawLimiter, StatusKind, ToolApprovalAction,
        WorkerUiEvent, split_layout,
    };
    #[allow(unused_imports)]
    use super::{ColorSupport, Theme, ThemeName};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::layout::Rect;
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn splits_into_four_panes() {
        let [conversation, worker, status, input] = split_layout(Rect::new(0, 0, 120, 40));

        assert_eq!(conversation.width + worker.width, 120);
        assert_eq!(status.height, 5);
        assert_eq!(input.height, 3);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = AppState::default();

        let action = app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(matches!(action, AppAction::Quit));
    }

    #[test]
    fn ctrl_v_inserts_image_placeholder_and_status() {
        let mut app = AppState::default();
        let action = app.handle_ctrl_v_with(|| Ok(PathBuf::from("/tmp/ccode-clipboard-123.png")));
        assert!(matches!(action, AppAction::None));
        assert_eq!(app.input.as_str(), "@image:/tmp/ccode-clipboard-123.png ");
        let status = app.render_status_lines();
        assert!(
            status
                .iter()
                .any(|line| line.to_string().contains("pasted image"))
        );
    }

    #[test]
    fn ctrl_v_error_shows_inline_status_without_panic() {
        let mut app = AppState::default();
        let action = app.handle_ctrl_v_with(|| Err("non-image content".to_string()));
        assert!(matches!(action, AppAction::None));
        assert_eq!(app.input.as_str(), "");
        let status = app.render_status_lines();
        assert!(
            status
                .iter()
                .any(|line| line.to_string().contains("clipboard paste failed"))
        );
    }

    #[test]
    fn appends_deltas_to_active_assistant_message() {
        let mut app = AppState::default();
        app.apply_delta("hello");
        app.apply_delta(" world");
        app.close_active_assistant();
        app.apply_delta("new");

        assert_eq!(app.conversation.len(), 2);
        match &app.conversation[0] {
            ConversationLine::Assistant(content) => assert_eq!(content, "hello world"),
            other => panic!("unexpected conversation entry: {other:?}"),
        }
        match &app.conversation[1] {
            ConversationLine::Assistant(content) => assert_eq!(content, "new"),
            other => panic!("unexpected conversation entry: {other:?}"),
        }
    }

    #[test]
    fn tool_timeline_marks_success_and_failure() {
        let mut app = AppState::default();
        app.push_tool_done("shell".to_string(), true);
        app.push_tool_done("shell".to_string(), false);

        let lines = app.render_status_lines();
        assert!(lines.iter().any(|l| l.to_string().contains("[ok]")));
        assert!(lines.iter().any(|l| l.to_string().contains("[fail]")));
    }

    #[test]
    fn conversation_scroll_navigation_changes_offset() {
        let mut app = AppState::default();
        app.conversation_scroll = 10;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
        assert_eq!(app.conversation_scroll, 9);
        let _ = app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.conversation_scroll, 19);
    }

    #[test]
    fn grapheme_aware_backspace_and_cursor_navigation() {
        let mut app = AppState::default();
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('🙂'), KeyModifiers::NONE));
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE));

        let _ = app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let _ = app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(app.input.as_str(), "a你");
        let _ = app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.input.as_str(), "a你");
    }

    #[test]
    fn up_down_recalls_input_history() {
        let mut app = AppState::default();
        app.input.set_text("first".to_string());
        assert!(matches!(app.submit_input(), AppAction::Submit(_)));
        app.input.set_text("second".to_string());
        assert!(matches!(app.submit_input(), AppAction::Submit(_)));

        let _ = app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.input.as_str(), "second");
        let _ = app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.input.as_str(), "first");
        let _ = app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.input.as_str(), "second");
    }

    #[test]
    fn enter_submits_but_shift_enter_inserts_newline() {
        let mut app = AppState::default();
        app.input.set_text("line one".to_string());
        let _ = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        app.input.insert_str("line two");

        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            AppAction::Submit(payload) => assert_eq!(payload, "line one\nline two"),
            _ => panic!("expected submit action"),
        }
    }

    #[test]
    fn ime_preedit_and_commit_do_not_submit_prematurely() {
        let mut app = AppState::default();
        app.ime_preedit_for_test("ni".to_string());
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::None
        ));

        app.ime_commit_for_test("你".to_string());
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::None
        ));
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::Submit(_)
        ));
    }

    #[test]
    fn input_cursor_position_accounts_for_fullwidth_and_wrapping() {
        let mut app = AppState::default();
        app.input.set_text("a你🙂".to_string());
        let pos = app.input_cursor_position(Rect::new(0, 0, 6, 4));
        assert_eq!(pos, (2, 2));
    }

    #[test]
    fn key_release_events_are_ignored() {
        let mut app = AppState::default();
        let key = KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::empty(),
        };
        let action = app.handle_key(key);
        assert!(matches!(action, AppAction::None));
        assert_eq!(app.input.as_str(), "");
    }

    #[test]
    fn error_status_uses_category_label() {
        let mut app = AppState::default();
        app.push_error_status("authentication failed with api key".to_string());
        let lines = app.render_status_lines();
        let rendered: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("[auth]"));
    }

    #[test]
    fn draw_limiter_throttles_high_frequency_redraws() {
        let now = Instant::now();
        assert!(!DrawLimiter::should_draw(now));

        let old = now - Duration::from_millis(30);
        assert!(DrawLimiter::should_draw(old));
        assert_eq!(DrawLimiter::next_timeout(old, true), Duration::ZERO);
    }

    #[test]
    fn info_status_category_renders() {
        let mut app = AppState::default();
        app.push_info_status("ready".to_string());
        assert!(matches!(app.status.back().unwrap().kind, StatusKind::Info));
    }

    #[test]
    fn tool_approval_modal_shows_and_denial_is_logged() {
        let mut app = AppState::default();
        app.open_tool_approval_for_test("shell".to_string(), json!({"cmd":"rm -rf /tmp/demo"}));

        assert!(app.has_pending_tool_approval());
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        match action {
            AppAction::ToolApprovalResolved {
                name,
                decision: ToolApprovalAction::Deny,
            } => assert_eq!(name, "shell"),
            other => panic!("expected deny decision, got {other:?}"),
        }
        assert!(!app.has_pending_tool_approval());
        assert!(
            app.render_conversation()
                .iter()
                .all(|line| !line.to_string().contains("[tool:decision]")),
            "tool decision should not be logged in conversation pane"
        );
    }

    #[test]
    fn tool_approval_modal_supports_tab_and_enter_shortcuts() {
        let mut app = AppState::default();
        app.open_tool_approval_for_test("fs_write".to_string(), json!({"path":"./a.txt"}));

        let _ = app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            AppAction::ToolApprovalResolved {
                decision: ToolApprovalAction::Deny,
                ..
            }
        ));
    }

    #[test]
    fn tool_approval_modal_supports_always_allow_shortcut() {
        let mut app = AppState::default();
        app.open_tool_approval_for_test(
            "browser".to_string(),
            json!({"url":"https://example.com"}),
        );

        let action = app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(
            action,
            AppAction::ToolApprovalResolved {
                decision: ToolApprovalAction::AllowAlways,
                ..
            }
        ));
    }

    #[test]
    fn worker_panel_tracks_task_details_and_status_badges() {
        let mut app = AppState::default();
        app.record_worker_event(WorkerUiEvent {
            task_id: "w-1".to_string(),
            status: "Running".to_string(),
            summary: Some("indexing".to_string()),
            timestamp: std::time::UNIX_EPOCH + Duration::from_secs(1),
        });
        app.record_worker_event(WorkerUiEvent {
            task_id: "w-1".to_string(),
            status: "Completed".to_string(),
            summary: Some("done".to_string()),
            timestamp: std::time::UNIX_EPOCH + Duration::from_secs(3),
        });

        let selected = app
            .selected_worker_task()
            .expect("selected worker task should exist");
        assert_eq!(selected.task_id, "w-1");
        assert_eq!(selected.status, "Completed");
        assert_eq!(selected.summary.as_deref(), Some("done"));
        assert_eq!(
            selected.started_at,
            std::time::UNIX_EPOCH + Duration::from_secs(1)
        );
        assert_eq!(
            selected.updated_at,
            std::time::UNIX_EPOCH + Duration::from_secs(3)
        );

        let panel_lines = app.render_worker_list(20, 80);
        assert!(
            panel_lines
                .iter()
                .any(|line| line.to_string().contains("[Completed]")),
            "worker list should render Completed status badge"
        );
    }

    #[test]
    fn worker_panel_selection_and_virtual_scroll_work_for_high_volume() {
        let mut app = AppState::default();
        for idx in 0..120u64 {
            app.record_worker_event(WorkerUiEvent {
                task_id: format!("w-{idx}"),
                status: "Running".to_string(),
                summary: Some(format!("task {idx}")),
                timestamp: std::time::UNIX_EPOCH + Duration::from_secs(idx),
            });
        }

        for _ in 0..80 {
            app.select_next_worker_task();
        }

        let selected = app
            .selected_worker_task()
            .expect("selected worker task should exist");
        assert_eq!(selected.task_id, "w-39");

        let lines = app.render_worker_list(10, 80);
        assert!(
            lines.iter().all(|line| !line.to_string().contains("w-119")),
            "virtual list should not render off-screen task rows"
        );
        assert!(
            lines.iter().any(|line| line.to_string().contains(">")),
            "selected row should be visible and marked"
        );
    }

    #[test]
    fn no_color_theme_uses_no_fg_colors() {
        use super::{ColorSupport, Theme, ThemeName};
        use ratatui::style::Color;
        let theme = Theme::build(ThemeName::NoColor, ColorSupport::Basic16);
        // no_color theme must not assign any foreground color
        assert!(
            theme.user_line.fg.is_none() || theme.user_line.fg == Some(Color::Reset),
            "no_color user_line must have no fg color"
        );
        assert!(
            theme.status_error.fg.is_none() || theme.status_error.fg == Some(Color::Reset),
            "no_color status_error must have no fg color"
        );
    }

    #[test]
    fn default_theme_with_no_color_env_produces_no_color_theme() {
        use super::{ColorSupport, Theme, ThemeName};
        use ratatui::style::Color;
        // Regardless of the configured theme name, ColorSupport::None forces no-color.
        let theme = Theme::build(ThemeName::Default, ColorSupport::None);
        assert!(
            theme.user_line.fg.is_none() || theme.user_line.fg == Some(Color::Reset),
            "ColorSupport::None must force no fg colors"
        );
    }

    #[test]
    fn default_theme_assigns_colors() {
        use super::{ColorSupport, Theme, ThemeName};
        use ratatui::style::Color;
        let theme = Theme::build(ThemeName::Default, ColorSupport::Basic16);
        assert_eq!(theme.user_line.fg, Some(Color::Cyan));
        assert_eq!(theme.assistant_line.fg, Some(Color::Green));
        assert_eq!(theme.status_error.fg, Some(Color::Red));
    }

    #[test]
    fn high_contrast_theme_assigns_bright_colors() {
        use super::{ColorSupport, Theme, ThemeName};
        use ratatui::style::Color;
        let theme = Theme::build(ThemeName::HighContrast, ColorSupport::Basic16);
        assert_eq!(theme.user_line.fg, Some(Color::LightCyan));
        assert_eq!(theme.assistant_line.fg, Some(Color::LightGreen));
        assert_eq!(theme.status_error.fg, Some(Color::LightRed));
    }

    #[test]
    fn render_status_lines_prefixes_info_and_error_indicators() {
        let mut app = AppState::default();
        app.push_info_status("all good".to_string());
        app.push_error_status("connection timeout".to_string());
        let lines = app.render_status_lines();
        let text: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        // Non-color indicators must be present regardless of theme
        assert!(text.contains("[*]"), "info indicator [*] must appear");
        assert!(text.contains("[!]"), "error indicator [!] must appear");
        // Text category labels must also be present
        assert!(text.contains("[info]"), "[info] label must appear");
        assert!(
            text.contains("[transport]"),
            "[transport] label must appear"
        );
    }

    #[test]
    fn render_status_includes_non_color_tool_markers() {
        let mut app = AppState::default();
        app.push_tool_done("shell".to_string(), true);
        app.push_tool_done("shell".to_string(), false);
        let lines = app.render_status_lines();
        let text: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("[ok]"), "[ok] marker must appear");
        assert!(text.contains("[fail]"), "[fail] marker must appear");
    }

    #[test]
    fn worker_list_hint_shows_key_hints_when_empty() {
        let app = AppState::default();
        let lines = app.render_worker_list(10, 80);
        let text: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("Alt+Up") || text.contains("Keys"),
            "key hints must be visible when worker panel is empty"
        );
    }
}
