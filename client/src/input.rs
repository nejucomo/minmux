use crate::app::AppState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use minmux_ipc::{ClientMessage, MoveDirection, NavigateDirection};

/// The current input mode of the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Command,
    Navigate,
    Move,
}

impl std::fmt::Display for InputMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputMode::Normal => write!(f, "normal"),
            InputMode::Command => write!(f, "command"),
            InputMode::Navigate => write!(f, "navigate"),
            InputMode::Move => write!(f, "move"),
        }
    }
}

/// Help text shown when the user presses `^T h`.
pub const HELP_TEXT: &str = r#"Key Bindings
============

Normal mode:
  ^T            Enter command mode
  <any>         Send key to active pane

Command mode:
  h             Show this help
  p             Append new pane
  n             Enter navigate mode
  m             Enter move mode
  d             Detach session
  <Escape>      Return to normal mode

Navigate mode (navigate between panes/windows):
  j             Pane below
  k             Pane above
  h             Window to the left (top pane)
  l             Window to the right (top pane)
  <Escape>      Return to normal mode

Move mode (move the active pane):
  j             Move pane down
  k             Move pane up
  h             Move pane left (or new window)
  l             Move pane right (or new window)
  <Escape>      Return to normal mode
"#;

/// Handle one key press.  Returns `(quit, Option<ClientMessage>)`.
pub fn handle_key(
    key: KeyEvent,
    state: &mut AppState,
) -> (bool, Option<ClientMessage>) {
    match &state.input_mode {
        InputMode::Normal => handle_normal(key, state),
        InputMode::Command => handle_non_normal(key, state, InputMode::Command),
        InputMode::Navigate => handle_non_normal(key, state, InputMode::Navigate),
        InputMode::Move => handle_non_normal(key, state, InputMode::Move),
    }
}

fn handle_normal(key: KeyEvent, state: &mut AppState) -> (bool, Option<ClientMessage>) {
    // ^T switches to command mode.
    if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('t') {
        state.input_mode = InputMode::Command;
        state.pending_sequence.clear();
        state.status_message = None;
        return (false, None);
    }

    // Forward everything else to the active pane.
    let msg = if let Some(data) = key_to_bytes(&key) {
        active_pane_id(state).map(|id| ClientMessage::SendInput { pane_id: id, data })
    } else {
        None
    };
    (false, msg)
}

fn handle_non_normal(
    key: KeyEvent,
    state: &mut AppState,
    mode: InputMode,
) -> (bool, Option<ClientMessage>) {
    // Escape always returns to normal mode.
    if key.code == KeyCode::Esc {
        state.input_mode = InputMode::Normal;
        state.pending_sequence.clear();
        state.status_message = None;
        state.help_text = None;
        return (false, None);
    }

    state.pending_sequence.push(key);

    match mode {
        InputMode::Command => handle_command(state),
        InputMode::Navigate => handle_navigate(state),
        InputMode::Move => handle_move(state),
        InputMode::Normal => unreachable!(),
    }
}

fn handle_command(state: &mut AppState) -> (bool, Option<ClientMessage>) {
    let key = match state.pending_sequence.last().cloned() {
        Some(k) => k,
        None => return (false, None),
    };

    match key.code {
        KeyCode::Char('h') => {
            state.help_text = Some(HELP_TEXT.to_string());
            state.pending_sequence.clear();
            (false, None)
        }
        KeyCode::Char('p') => {
            state.pending_sequence.clear();
            let msg = active_window_id(state).map(|wid| ClientMessage::CreatePane {
                window_id: wid,
                command: None,
            });
            (false, msg)
        }
        KeyCode::Char('n') => {
            state.input_mode = InputMode::Navigate;
            state.pending_sequence.clear();
            state.status_message = Some("navigate mode".to_string());
            (false, None)
        }
        KeyCode::Char('m') => {
            state.input_mode = InputMode::Move;
            state.pending_sequence.clear();
            state.status_message = Some("move mode".to_string());
            (false, None)
        }
        KeyCode::Char('d') => {
            state.pending_sequence.clear();
            (false, Some(ClientMessage::Detach))
        }
        _ => {
            state.status_message = Some(format!(
                "unknown command: {}",
                key_display(&key)
            ));
            state.pending_sequence.clear();
            (false, None)
        }
    }
}

fn handle_navigate(state: &mut AppState) -> (bool, Option<ClientMessage>) {
    let key = match state.pending_sequence.last().cloned() {
        Some(k) => k,
        None => return (false, None),
    };
    state.pending_sequence.clear();

    let session_name = match &state.session {
        Some(s) => s.name.clone(),
        None => return (false, None),
    };

    let direction = match key.code {
        KeyCode::Char('j') => NavigateDirection::Down,
        KeyCode::Char('k') => NavigateDirection::Up,
        KeyCode::Char('h') => NavigateDirection::Left,
        KeyCode::Char('l') => NavigateDirection::Right,
        _ => {
            state.status_message = Some(format!(
                "unknown navigate key: {}",
                key_display(&key)
            ));
            return (false, None);
        }
    };

    (
        false,
        Some(ClientMessage::Navigate {
            session_name,
            direction,
        }),
    )
}

fn handle_move(state: &mut AppState) -> (bool, Option<ClientMessage>) {
    let key = match state.pending_sequence.last().cloned() {
        Some(k) => k,
        None => return (false, None),
    };
    state.pending_sequence.clear();

    let pane_id = match active_pane_id(state) {
        Some(id) => id,
        None => return (false, None),
    };

    let direction = match key.code {
        KeyCode::Char('j') => MoveDirection::Down,
        KeyCode::Char('k') => MoveDirection::Up,
        KeyCode::Char('h') => MoveDirection::Left,
        KeyCode::Char('l') => MoveDirection::Right,
        _ => {
            state.status_message = Some(format!(
                "unknown move key: {}",
                key_display(&key)
            ));
            return (false, None);
        }
    };

    (
        false,
        Some(ClientMessage::MovePane { pane_id, direction }),
    )
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn active_pane_id(state: &AppState) -> Option<minmux_ipc::PaneId> {
    let session = state.session.as_ref()?;
    let win = session
        .windows
        .iter()
        .find(|w| w.id == session.active_window_id)?;
    Some(win.active_pane_id)
}

fn active_window_id(state: &AppState) -> Option<minmux_ipc::WindowId> {
    let session = state.session.as_ref()?;
    Some(session.active_window_id)
}

fn key_to_bytes(key: &KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers == KeyModifiers::CONTROL {
                let byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                Some(vec![byte])
            } else {
                let mut buf = [0u8; 4];
                Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
            }
        }
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Backspace => Some(b"\x7f".to_vec()),
        KeyCode::Tab => Some(b"\t".to_vec()),
        KeyCode::Esc => Some(b"\x1b".to_vec()),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        _ => None,
    }
}

fn key_display(key: &KeyEvent) -> String {
    if key.modifiers == KeyModifiers::CONTROL {
        if let KeyCode::Char(c) = key.code {
            return format!("^{}", c.to_ascii_uppercase());
        }
    }
    match key.code {
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}
