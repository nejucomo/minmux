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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use minmux_ipc::{PaneInfo, SessionInfo, WindowInfo};

    // ─── Key-event constructors ───────────────────────────────────────────────

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // ─── AppState constructors ────────────────────────────────────────────────

    fn state_with_session() -> AppState {
        AppState {
            session: Some(SessionInfo {
                name: "test".to_string(),
                windows: vec![WindowInfo {
                    id: 1,
                    name: "w1".to_string(),
                    panes: vec![PaneInfo {
                        id: 10,
                        name: "p1".to_string(),
                    }],
                    active_pane_id: 10,
                }],
                active_window_id: 1,
            }),
            input_mode: InputMode::Normal,
            pending_sequence: Vec::new(),
            status_message: None,
            should_quit: false,
            help_text: None,
        }
    }

    fn state_no_session() -> AppState {
        AppState {
            session: None,
            input_mode: InputMode::Normal,
            pending_sequence: Vec::new(),
            status_message: None,
            should_quit: false,
            help_text: None,
        }
    }

    fn enter_command(state: &mut AppState) {
        let (_, _) = handle_key(ctrl('t'), state);
        assert_eq!(state.input_mode, InputMode::Command);
    }

    fn enter_navigate(state: &mut AppState) {
        enter_command(state);
        handle_key(press(KeyCode::Char('n')), state);
        assert_eq!(state.input_mode, InputMode::Navigate);
    }

    fn enter_move(state: &mut AppState) {
        enter_command(state);
        handle_key(press(KeyCode::Char('m')), state);
        assert_eq!(state.input_mode, InputMode::Move);
    }

    // ─── InputMode Display ────────────────────────────────────────────────────

    #[test]
    fn input_mode_display_values() {
        assert_eq!(InputMode::Normal.to_string(), "normal");
        assert_eq!(InputMode::Command.to_string(), "command");
        assert_eq!(InputMode::Navigate.to_string(), "navigate");
        assert_eq!(InputMode::Move.to_string(), "move");
    }

    // ─── Normal mode ──────────────────────────────────────────────────────────

    #[test]
    fn normal_ctrl_t_enters_command_mode() {
        let mut state = state_with_session();
        let (quit, msg) = handle_key(ctrl('t'), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        assert_eq!(state.input_mode, InputMode::Command);
    }

    #[test]
    fn normal_ctrl_t_clears_pending_sequence() {
        let mut state = state_with_session();
        state.pending_sequence.push(press(KeyCode::Char('x')));
        handle_key(ctrl('t'), &mut state);
        assert!(state.pending_sequence.is_empty());
    }

    #[test]
    fn normal_regular_char_sends_input_to_active_pane() {
        let mut state = state_with_session();
        let (quit, msg) = handle_key(press(KeyCode::Char('a')), &mut state);
        assert!(!quit);
        assert_eq!(state.input_mode, InputMode::Normal);
        match msg {
            Some(ClientMessage::SendInput { pane_id, data }) => {
                assert_eq!(pane_id, 10);
                assert_eq!(data, b"a");
            }
            other => panic!("expected SendInput, got {other:?}"),
        }
    }

    #[test]
    fn normal_regular_char_without_session_produces_no_message() {
        let mut state = state_no_session();
        let (quit, msg) = handle_key(press(KeyCode::Char('x')), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
    }

    #[test]
    fn normal_enter_sends_carriage_return() {
        let mut state = state_with_session();
        let (_, msg) = handle_key(press(KeyCode::Enter), &mut state);
        match msg {
            Some(ClientMessage::SendInput { data, .. }) => assert_eq!(data, b"\r"),
            other => panic!("expected SendInput, got {other:?}"),
        }
    }

    #[test]
    fn normal_escape_sends_escape_byte_not_mode_change() {
        let mut state = state_with_session();
        let (_, msg) = handle_key(press(KeyCode::Esc), &mut state);
        // In Normal mode Esc is forwarded to the pane, not consumed as a mode switch.
        match msg {
            Some(ClientMessage::SendInput { data, .. }) => assert_eq!(data, b"\x1b"),
            other => panic!("expected SendInput, got {other:?}"),
        }
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn normal_arrow_up_sends_escape_sequence() {
        let mut state = state_with_session();
        let (_, msg) = handle_key(press(KeyCode::Up), &mut state);
        match msg {
            Some(ClientMessage::SendInput { data, .. }) => assert_eq!(data, b"\x1b[A"),
            other => panic!("expected SendInput, got {other:?}"),
        }
    }

    // ─── Command mode ─────────────────────────────────────────────────────────

    #[test]
    fn command_esc_returns_to_normal_and_clears_state() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Esc), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.pending_sequence.is_empty());
        assert!(state.status_message.is_none());
        assert!(state.help_text.is_none());
    }

    #[test]
    fn command_p_issues_create_pane_for_active_window() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('p')), &mut state);
        assert!(!quit);
        match msg {
            Some(ClientMessage::CreatePane { window_id, command }) => {
                assert_eq!(window_id, 1);
                assert!(command.is_none());
            }
            other => panic!("expected CreatePane, got {other:?}"),
        }
    }

    #[test]
    fn command_p_without_session_produces_no_message() {
        let mut state = state_no_session();
        state.input_mode = InputMode::Command;
        let (_, msg) = handle_key(press(KeyCode::Char('p')), &mut state);
        assert!(msg.is_none());
    }

    #[test]
    fn command_n_enters_navigate_mode() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('n')), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        assert_eq!(state.input_mode, InputMode::Navigate);
    }

    #[test]
    fn command_m_enters_move_mode() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('m')), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        assert_eq!(state.input_mode, InputMode::Move);
    }

    #[test]
    fn command_d_sends_detach() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('d')), &mut state);
        assert!(!quit);
        assert!(matches!(msg, Some(ClientMessage::Detach)));
    }

    #[test]
    fn command_h_sets_help_text() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('h')), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        let help = state.help_text.expect("help_text should be set");
        assert!(help.contains("Key Bindings"));
    }

    #[test]
    fn command_unknown_key_sets_error_message() {
        let mut state = state_with_session();
        enter_command(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('z')), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        let err = state.status_message.expect("status_message should be set");
        assert!(err.contains("unknown command"), "unexpected: {err}");
    }

    // ─── Navigate mode ────────────────────────────────────────────────────────

    #[test]
    fn navigate_j_sends_navigate_down() {
        let mut state = state_with_session();
        enter_navigate(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('j')), &mut state);
        assert!(!quit);
        match msg {
            Some(ClientMessage::Navigate { session_name, direction }) => {
                assert_eq!(session_name, "test");
                assert!(matches!(direction, NavigateDirection::Down));
            }
            other => panic!("expected Navigate Down, got {other:?}"),
        }
    }

    #[test]
    fn navigate_k_sends_navigate_up() {
        let mut state = state_with_session();
        enter_navigate(&mut state);
        let (_, msg) = handle_key(press(KeyCode::Char('k')), &mut state);
        assert!(
            matches!(msg, Some(ClientMessage::Navigate { direction: NavigateDirection::Up, .. }))
        );
    }

    #[test]
    fn navigate_h_sends_navigate_left() {
        let mut state = state_with_session();
        enter_navigate(&mut state);
        let (_, msg) = handle_key(press(KeyCode::Char('h')), &mut state);
        assert!(
            matches!(msg, Some(ClientMessage::Navigate { direction: NavigateDirection::Left, .. }))
        );
    }

    #[test]
    fn navigate_l_sends_navigate_right() {
        let mut state = state_with_session();
        enter_navigate(&mut state);
        let (_, msg) = handle_key(press(KeyCode::Char('l')), &mut state);
        assert!(
            matches!(msg, Some(ClientMessage::Navigate { direction: NavigateDirection::Right, .. }))
        );
    }

    #[test]
    fn navigate_esc_returns_to_normal() {
        let mut state = state_with_session();
        enter_navigate(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Esc), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn navigate_unknown_key_sets_error_stays_in_navigate() {
        let mut state = state_with_session();
        enter_navigate(&mut state);
        handle_key(press(KeyCode::Char('z')), &mut state);
        let err = state.status_message.as_deref().unwrap_or("");
        assert!(err.contains("unknown navigate key"), "unexpected: {err}");
    }

    #[test]
    fn navigate_without_session_produces_no_message() {
        let mut state = state_no_session();
        state.input_mode = InputMode::Navigate;
        let (_, msg) = handle_key(press(KeyCode::Char('j')), &mut state);
        assert!(msg.is_none());
    }

    // ─── Move mode ────────────────────────────────────────────────────────────

    #[test]
    fn move_j_sends_move_pane_down() {
        let mut state = state_with_session();
        enter_move(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Char('j')), &mut state);
        assert!(!quit);
        match msg {
            Some(ClientMessage::MovePane { pane_id, direction }) => {
                assert_eq!(pane_id, 10);
                assert!(matches!(direction, MoveDirection::Down));
            }
            other => panic!("expected MovePane Down, got {other:?}"),
        }
    }

    #[test]
    fn move_k_sends_move_pane_up() {
        let mut state = state_with_session();
        enter_move(&mut state);
        let (_, msg) = handle_key(press(KeyCode::Char('k')), &mut state);
        assert!(
            matches!(msg, Some(ClientMessage::MovePane { direction: MoveDirection::Up, .. }))
        );
    }

    #[test]
    fn move_h_sends_move_pane_left() {
        let mut state = state_with_session();
        enter_move(&mut state);
        let (_, msg) = handle_key(press(KeyCode::Char('h')), &mut state);
        assert!(
            matches!(msg, Some(ClientMessage::MovePane { direction: MoveDirection::Left, .. }))
        );
    }

    #[test]
    fn move_l_sends_move_pane_right() {
        let mut state = state_with_session();
        enter_move(&mut state);
        let (_, msg) = handle_key(press(KeyCode::Char('l')), &mut state);
        assert!(
            matches!(msg, Some(ClientMessage::MovePane { direction: MoveDirection::Right, .. }))
        );
    }

    #[test]
    fn move_esc_returns_to_normal() {
        let mut state = state_with_session();
        enter_move(&mut state);
        let (quit, msg) = handle_key(press(KeyCode::Esc), &mut state);
        assert!(!quit);
        assert!(msg.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn move_unknown_key_sets_error() {
        let mut state = state_with_session();
        enter_move(&mut state);
        handle_key(press(KeyCode::Char('z')), &mut state);
        let err = state.status_message.as_deref().unwrap_or("");
        assert!(err.contains("unknown move key"), "unexpected: {err}");
    }

    #[test]
    fn move_without_session_produces_no_message() {
        let mut state = state_no_session();
        state.input_mode = InputMode::Move;
        let (_, msg) = handle_key(press(KeyCode::Char('j')), &mut state);
        assert!(msg.is_none());
    }

    // ─── key_to_bytes ─────────────────────────────────────────────────────────

    #[test]
    fn key_to_bytes_regular_printable_char() {
        assert_eq!(key_to_bytes(&press(KeyCode::Char('a'))), Some(b"a".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::Char('Z'))), Some(b"Z".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::Char('5'))), Some(b"5".to_vec()));
    }

    #[test]
    fn key_to_bytes_multibyte_unicode_char() {
        let key = press(KeyCode::Char('é'));
        let bytes = key_to_bytes(&key).expect("should produce bytes");
        assert_eq!(bytes, "é".as_bytes());
    }

    #[test]
    fn key_to_bytes_ctrl_a_through_z() {
        // Ctrl+A = 0x01, Ctrl+B = 0x02, …, Ctrl+Z = 0x1A
        for (i, c) in ('a'..='z').enumerate() {
            let expected = (i + 1) as u8;
            let key = ctrl(c);
            let bytes = key_to_bytes(&key).expect("ctrl char should produce bytes");
            assert_eq!(bytes, vec![expected], "ctrl+{c} should be 0x{expected:02X}");
        }
    }

    #[test]
    fn key_to_bytes_enter_is_cr() {
        assert_eq!(key_to_bytes(&press(KeyCode::Enter)), Some(b"\r".to_vec()));
    }

    #[test]
    fn key_to_bytes_backspace_is_del() {
        assert_eq!(key_to_bytes(&press(KeyCode::Backspace)), Some(b"\x7f".to_vec()));
    }

    #[test]
    fn key_to_bytes_tab() {
        assert_eq!(key_to_bytes(&press(KeyCode::Tab)), Some(b"\t".to_vec()));
    }

    #[test]
    fn key_to_bytes_esc() {
        assert_eq!(key_to_bytes(&press(KeyCode::Esc)), Some(b"\x1b".to_vec()));
    }

    #[test]
    fn key_to_bytes_arrow_keys() {
        assert_eq!(key_to_bytes(&press(KeyCode::Up)), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::Down)), Some(b"\x1b[B".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::Right)), Some(b"\x1b[C".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::Left)), Some(b"\x1b[D".to_vec()));
    }

    #[test]
    fn key_to_bytes_home_end() {
        assert_eq!(key_to_bytes(&press(KeyCode::Home)), Some(b"\x1b[H".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::End)), Some(b"\x1b[F".to_vec()));
    }

    #[test]
    fn key_to_bytes_delete() {
        assert_eq!(key_to_bytes(&press(KeyCode::Delete)), Some(b"\x1b[3~".to_vec()));
    }

    #[test]
    fn key_to_bytes_page_up_down() {
        assert_eq!(key_to_bytes(&press(KeyCode::PageUp)), Some(b"\x1b[5~".to_vec()));
        assert_eq!(key_to_bytes(&press(KeyCode::PageDown)), Some(b"\x1b[6~".to_vec()));
    }

    #[test]
    fn key_to_bytes_unrecognised_key_returns_none() {
        assert!(key_to_bytes(&press(KeyCode::F(1))).is_none());
        assert!(key_to_bytes(&press(KeyCode::F(12))).is_none());
        assert!(key_to_bytes(&press(KeyCode::Insert)).is_none());
    }
}
