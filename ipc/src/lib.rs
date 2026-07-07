use serde::{Deserialize, Serialize};

pub mod connection;

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
