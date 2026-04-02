use clap::Args;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io;
use std::time::Duration;

#[derive(Args, Default)]
pub struct TuiArgs {}

pub async fn run(_args: TuiArgs) -> anyhow::Result<()> {
    run_ui_loop()
}

fn run_ui_loop() -> anyhow::Result<()> {
    install_panic_restoration_hook();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let terminal_guard = TerminalGuard;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = AppState::default();
    while !app.should_quit {
        terminal.draw(|frame| draw_ui(frame, &app))?;

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key);
        }
    }

    terminal.show_cursor()?;
    drop(terminal);
    drop(terminal_guard);
    Ok(())
}

fn draw_ui(frame: &mut Frame<'_>, app: &AppState) {
    let [conversation_pane, status_pane, input_pane] = split_layout(frame.area());

    let conversation = Paragraph::new(app.conversation.join("\n"))
        .block(
            Block::default()
                .title("Conversation")
                .borders(Borders::ALL)
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, conversation_pane);

    let status = Paragraph::new(app.status.as_str()).block(
        Block::default()
            .title("Status")
            .borders(Borders::ALL)
            .title_style(Style::default().add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(status, status_pane);

    let input = Paragraph::new(app.input.as_str()).block(
        Block::default()
            .title("Input")
            .borders(Borders::ALL)
            .title_style(Style::default().add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(input, input_pane);
}

fn split_layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(3),
    ])
    .areas(area)
}

#[derive(Default)]
struct AppState {
    conversation: Vec<String>,
    status: String,
    input: String,
    should_quit: bool,
}

impl AppState {
    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match key.code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('q') if self.input.is_empty() => self.should_quit = true,
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn submit_input(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }
        if matches!(input.as_str(), "q" | "quit" | "exit") {
            self.should_quit = true;
            return;
        }

        self.conversation.push(format!("You: {input}"));
        self.conversation
            .push("Agent: [TUI foundation active]".to_string());
        self.status = format!("messages: {}", self.conversation.len() / 2);
        self.input.clear();
    }
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
    use super::{AppState, split_layout};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    #[test]
    fn splits_into_three_panes() {
        let [conversation, status, input] = split_layout(Rect::new(0, 0, 120, 40));

        assert_eq!(conversation.width, 120);
        assert_eq!(status.height, 3);
        assert_eq!(input.height, 3);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = AppState::default();

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(app.should_quit);
    }
}
