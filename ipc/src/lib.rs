use serde::{Deserialize, Serialize};

pub mod connection;

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn sample_session_info() -> SessionInfo {
        SessionInfo {
            name: "my-session".to_string(),
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
        }
    }

    fn roundtrip<T>(value: &T) -> T
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let json = serde_json::to_string(value).expect("serialise");
        serde_json::from_str(&json).expect("deserialise")
    }

    // ─── PaneInfo / WindowInfo / SessionInfo ─────────────────────────────────

    #[test]
    fn pane_info_roundtrip() {
        let original = PaneInfo {
            id: 42,
            name: "my-pane".to_string(),
        };
        let back: PaneInfo = roundtrip(&original);
        assert_eq!(back.id, original.id);
        assert_eq!(back.name, original.name);
    }

    #[test]
    fn window_info_roundtrip() {
        let original = WindowInfo {
            id: 7,
            name: "main".to_string(),
            panes: vec![PaneInfo {
                id: 1,
                name: "p1".to_string(),
            }],
            active_pane_id: 1,
        };
        let back: WindowInfo = roundtrip(&original);
        assert_eq!(back.id, original.id);
        assert_eq!(back.name, original.name);
        assert_eq!(back.panes.len(), 1);
        assert_eq!(back.panes[0].id, 1);
        assert_eq!(back.active_pane_id, 1);
    }

    #[test]
    fn window_info_multiple_panes_roundtrip() {
        let original = WindowInfo {
            id: 3,
            name: "multi".to_string(),
            panes: vec![
                PaneInfo { id: 1, name: "p1".to_string() },
                PaneInfo { id: 2, name: "p2".to_string() },
                PaneInfo { id: 3, name: "p3".to_string() },
            ],
            active_pane_id: 2,
        };
        let back: WindowInfo = roundtrip(&original);
        assert_eq!(back.panes.len(), 3);
        assert_eq!(back.active_pane_id, 2);
    }

    #[test]
    fn session_info_roundtrip() {
        let original = sample_session_info();
        let back: SessionInfo = roundtrip(&original);
        assert_eq!(back.name, "my-session");
        assert_eq!(back.windows.len(), 1);
        assert_eq!(back.active_window_id, 1);
    }

    #[test]
    fn session_info_multiple_windows_roundtrip() {
        let original = SessionInfo {
            name: "s".to_string(),
            windows: vec![
                WindowInfo {
                    id: 1,
                    name: "w1".to_string(),
                    panes: vec![PaneInfo { id: 10, name: "p10".to_string() }],
                    active_pane_id: 10,
                },
                WindowInfo {
                    id: 2,
                    name: "w2".to_string(),
                    panes: vec![PaneInfo { id: 20, name: "p20".to_string() }],
                    active_pane_id: 20,
                },
            ],
            active_window_id: 2,
        };
        let back: SessionInfo = roundtrip(&original);
        assert_eq!(back.windows.len(), 2);
        assert_eq!(back.active_window_id, 2);
    }

    // ─── MoveDirection / NavigateDirection ────────────────────────────────────

    #[test]
    fn move_direction_all_variants_roundtrip() {
        for dir in [
            MoveDirection::Up,
            MoveDirection::Down,
            MoveDirection::Left,
            MoveDirection::Right,
        ] {
            let back: MoveDirection = roundtrip(&dir);
            // Compare via JSON string to avoid requiring PartialEq on the enum.
            assert_eq!(
                serde_json::to_string(&back).unwrap(),
                serde_json::to_string(&dir).unwrap()
            );
        }
    }

    #[test]
    fn navigate_direction_all_variants_roundtrip() {
        for dir in [
            NavigateDirection::Up,
            NavigateDirection::Down,
            NavigateDirection::Left,
            NavigateDirection::Right,
        ] {
            let back: NavigateDirection = roundtrip(&dir);
            assert_eq!(
                serde_json::to_string(&back).unwrap(),
                serde_json::to_string(&dir).unwrap()
            );
        }
    }

    // ─── ClientMessage ────────────────────────────────────────────────────────

    #[test]
    fn client_message_attach_roundtrip() {
        let msg = ClientMessage::Attach {
            session_name: "mysession".to_string(),
        };
        let back: ClientMessage = roundtrip(&msg);
        match back {
            ClientMessage::Attach { session_name } => assert_eq!(session_name, "mysession"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn client_message_create_pane_with_command_roundtrip() {
        let msg = ClientMessage::CreatePane {
            window_id: 3,
            command: Some("bash".to_string()),
        };
        let back: ClientMessage = roundtrip(&msg);
        match back {
            ClientMessage::CreatePane { window_id, command } => {
                assert_eq!(window_id, 3);
                assert_eq!(command, Some("bash".to_string()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_message_create_pane_no_command_roundtrip() {
        let msg = ClientMessage::CreatePane {
            window_id: 5,
            command: None,
        };
        let back: ClientMessage = roundtrip(&msg);
        match back {
            ClientMessage::CreatePane { window_id, command } => {
                assert_eq!(window_id, 5);
                assert!(command.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_message_move_pane_all_directions_roundtrip() {
        for dir in [
            MoveDirection::Up,
            MoveDirection::Down,
            MoveDirection::Left,
            MoveDirection::Right,
        ] {
            let msg = ClientMessage::MovePane {
                pane_id: 99,
                direction: dir,
            };
            let back: ClientMessage = roundtrip(&msg);
            match back {
                ClientMessage::MovePane { pane_id, .. } => assert_eq!(pane_id, 99),
                other => panic!("unexpected: {other:?}"),
            }
        }
    }

    #[test]
    fn client_message_navigate_all_directions_roundtrip() {
        for dir in [
            NavigateDirection::Up,
            NavigateDirection::Down,
            NavigateDirection::Left,
            NavigateDirection::Right,
        ] {
            let msg = ClientMessage::Navigate {
                session_name: "s".to_string(),
                direction: dir,
            };
            let back: ClientMessage = roundtrip(&msg);
            match back {
                ClientMessage::Navigate { session_name, .. } => assert_eq!(session_name, "s"),
                other => panic!("unexpected: {other:?}"),
            }
        }
    }

    #[test]
    fn client_message_send_input_roundtrip() {
        let msg = ClientMessage::SendInput {
            pane_id: 5,
            data: vec![0x01, 0x02, 0x03, 0xFF],
        };
        let back: ClientMessage = roundtrip(&msg);
        match back {
            ClientMessage::SendInput { pane_id, data } => {
                assert_eq!(pane_id, 5);
                assert_eq!(data, vec![0x01, 0x02, 0x03, 0xFF]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_message_send_input_empty_data_roundtrip() {
        let msg = ClientMessage::SendInput {
            pane_id: 1,
            data: vec![],
        };
        let back: ClientMessage = roundtrip(&msg);
        match back {
            ClientMessage::SendInput { data, .. } => assert!(data.is_empty()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_message_resize_pane_roundtrip() {
        let msg = ClientMessage::ResizePane {
            pane_id: 2,
            cols: 80,
            rows: 24,
        };
        let back: ClientMessage = roundtrip(&msg);
        match back {
            ClientMessage::ResizePane { pane_id, cols, rows } => {
                assert_eq!(pane_id, 2);
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_message_detach_roundtrip() {
        let msg = ClientMessage::Detach;
        let back: ClientMessage = roundtrip(&msg);
        assert!(matches!(back, ClientMessage::Detach));
    }

    // ─── ServerMessage ────────────────────────────────────────────────────────

    #[test]
    fn server_message_session_state_roundtrip() {
        let msg = ServerMessage::SessionState(sample_session_info());
        let back: ServerMessage = roundtrip(&msg);
        match back {
            ServerMessage::SessionState(info) => {
                assert_eq!(info.name, "my-session");
                assert_eq!(info.windows.len(), 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn server_message_pane_output_roundtrip() {
        let msg = ServerMessage::PaneOutput {
            pane_id: 9,
            data: b"hello world".to_vec(),
        };
        let back: ServerMessage = roundtrip(&msg);
        match back {
            ServerMessage::PaneOutput { pane_id, data } => {
                assert_eq!(pane_id, 9);
                assert_eq!(data, b"hello world");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn server_message_pane_output_binary_roundtrip() {
        let data: Vec<u8> = (0u8..=255).collect();
        let msg = ServerMessage::PaneOutput { pane_id: 1, data: data.clone() };
        let back: ServerMessage = roundtrip(&msg);
        match back {
            ServerMessage::PaneOutput { data: back_data, .. } => assert_eq!(back_data, data),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn server_message_error_roundtrip() {
        let msg = ServerMessage::Error {
            message: "something went wrong".to_string(),
        };
        let back: ServerMessage = roundtrip(&msg);
        match back {
            ServerMessage::Error { message } => {
                assert_eq!(message, "something went wrong");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn server_message_detached_roundtrip() {
        let msg = ServerMessage::Detached;
        let back: ServerMessage = roundtrip(&msg);
        assert!(matches!(back, ServerMessage::Detached));
    }

    // ─── Failure cases ────────────────────────────────────────────────────────

    #[test]
    fn deserialise_unknown_client_message_variant_fails() {
        let json = r#"{"UnknownVariant":{}}"#;
        let result = serde_json::from_str::<ClientMessage>(json);
        assert!(result.is_err());
    }

    #[test]
    fn deserialise_unknown_server_message_variant_fails() {
        let json = r#""UnknownVariant""#;
        let result = serde_json::from_str::<ServerMessage>(json);
        assert!(result.is_err());
    }

    #[test]
    fn deserialise_malformed_json_client_message_fails() {
        let result = serde_json::from_str::<ClientMessage>("not valid json at all");
        assert!(result.is_err());
    }

    #[test]
    fn deserialise_malformed_json_server_message_fails() {
        let result = serde_json::from_str::<ServerMessage>("{unclosed");
        assert!(result.is_err());
    }

    #[test]
    fn deserialise_missing_required_field_fails() {
        // Attach requires session_name; omit it.
        let json = r#"{"Attach":{}}"#;
        let result = serde_json::from_str::<ClientMessage>(json);
        assert!(result.is_err());
    }

    #[test]
    fn deserialise_wrong_type_for_field_fails() {
        // pane_id should be u64, not a string.
        let json = r#"{"ResizePane":{"pane_id":"not-a-number","cols":80,"rows":24}}"#;
        let result = serde_json::from_str::<ClientMessage>(json);
        assert!(result.is_err());
    }
}

pub use connection::Connection;

/// Unique identifier for a pane.
pub type PaneId = u64;

/// Unique identifier for a window.
pub type WindowId = u64;

/// Information about a pane sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: PaneId,
    pub name: String,
}

/// Information about a window sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub name: String,
    pub panes: Vec<PaneInfo>,
    pub active_pane_id: PaneId,
}

/// Information about a session sent to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub name: String,
    pub windows: Vec<WindowInfo>,
    pub active_window_id: WindowId,
}

/// Messages sent from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Attach to the named session (creating it if absent).
    Attach { session_name: String },
    /// Create a new pane in the given window.
    CreatePane {
        window_id: WindowId,
        command: Option<String>,
    },
    /// Move the active pane in the given direction.
    MovePane {
        pane_id: PaneId,
        direction: MoveDirection,
    },
    /// Navigate (change active pane) in the given direction.
    Navigate {
        session_name: String,
        direction: NavigateDirection,
    },
    /// Send raw input bytes to a pane's pseudo-terminal.
    SendInput { pane_id: PaneId, data: Vec<u8> },
    /// Resize a pane's pseudo-terminal.
    ResizePane {
        pane_id: PaneId,
        cols: u16,
        rows: u16,
    },
    /// Detach from the session without stopping it.
    Detach,
}

/// Directions for moving a pane.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MoveDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Directions for navigating between panes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NavigateDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Messages sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Current session state (sent after Attach and after any structural change).
    SessionState(SessionInfo),
    /// Output bytes from a pane's pseudo-terminal.
    PaneOutput { pane_id: PaneId, data: Vec<u8> },
    /// An error message to display to the user.
    Error { message: String },
    /// Acknowledgement that the client has been detached.
    Detached,
}
