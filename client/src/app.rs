use crate::input::{handle_key, InputMode};
use crate::ui::draw;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use minmux_ipc::{ClientMessage, Connection, ServerMessage, SessionInfo};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::sync::Mutex;

/// Application state shared between the draw loop and the network receiver.
pub struct AppState {
    pub session: Option<SessionInfo>,
    pub input_mode: InputMode,
    pub pending_sequence: Vec<crossterm::event::KeyEvent>,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub help_text: Option<String>,
}

impl AppState {
    fn new() -> Self {
        AppState {
            session: None,
            input_mode: InputMode::Normal,
            pending_sequence: Vec::new(),
            status_message: None,
            should_quit: false,
            help_text: None,
        }
    }
}

pub async fn run(socket_path: &Path, session_name: &str) -> Result<()> {
    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Connect to the daemon.
    let conn = Arc::new(Connection::connect(socket_path).await?);
    conn.send(&ClientMessage::Attach {
        session_name: session_name.to_string(),
    })
    .await?;

    let state = Arc::new(Mutex::new(AppState::new()));

    // Spawn a task to receive server messages.
    let state_recv = Arc::clone(&state);
    let conn_recv = Arc::clone(&conn);
    let _recv_task = tokio::spawn(async move {
        loop {
            match conn_recv.recv::<ServerMessage>().await {
                Ok(msg) => {
                    let mut s = state_recv.lock().await;
                    match msg {
                        ServerMessage::SessionState(info) => {
                            s.session = Some(info);
                        }
                        ServerMessage::PaneOutput { pane_id: _, data: _ } => {
                            // Pane output rendering via vt100 emulation is a future enhancement.
                        }
                        ServerMessage::Error { message } => {
                            s.status_message = Some(format!("Error: {message}"));
                        }
                        ServerMessage::Detached => {
                            s.should_quit = true;
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Main event loop.
    loop {
        // Draw.
        {
            let s = state.lock().await;
            terminal.draw(|f| draw(f, &s))?;
        }

        // Poll for keyboard input with a short timeout.
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let (quit, cmd_msg) = {
                        let mut s = state.lock().await;
                        handle_key(key, &mut s)
                    };

                    if let Some(msg) = cmd_msg {
                        conn.send(&msg).await?;
                    }

                    if quit {
                        break;
                    }
                }
            }
        }

        // Check if the receive task signalled quit.
        {
            let s = state.lock().await;
            if s.should_quit {
                break;
            }
        }
    }

    // Restore terminal.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
