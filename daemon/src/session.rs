use anyhow::{anyhow, Result};
use minmux_ipc::{
    MoveDirection, NavigateDirection, PaneId, PaneInfo, SessionInfo, WindowId, WindowInfo,
};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::Write;
use tokio::sync::mpsc;

// ─── Pane ────────────────────────────────────────────────────────────────────

struct Pane {
    id: PaneId,
    name: String,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
}

impl Pane {
    fn info(&self) -> PaneInfo {
        PaneInfo {
            id: self.id,
            name: self.name.clone(),
        }
    }
}

// ─── Window ──────────────────────────────────────────────────────────────────

struct Window {
    id: WindowId,
    name: String,
    panes: Vec<Pane>,
    active_pane_idx: usize,
}

impl Window {
    fn info(&self) -> WindowInfo {
        WindowInfo {
            id: self.id,
            name: self.name.clone(),
            panes: self.panes.iter().map(|p| p.info()).collect(),
            active_pane_id: self.panes[self.active_pane_idx].id,
        }
    }
}

// ─── Session ─────────────────────────────────────────────────────────────────

struct Session {
    name: String,
    windows: Vec<Window>,
    active_window_idx: usize,
}

impl Session {
    fn info(&self) -> SessionInfo {
        SessionInfo {
            name: self.name.clone(),
            windows: self.windows.iter().map(|w| w.info()).collect(),
            active_window_id: self.windows[self.active_window_idx].id,
        }
    }

    fn active_window_mut(&mut self) -> &mut Window {
        &mut self.windows[self.active_window_idx]
    }

    fn find_window_idx_by_id(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    fn find_window_idx_containing_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.windows
            .iter()
            .position(|w| w.panes.iter().any(|p| p.id == pane_id))
    }
}

// ─── SessionStore ─────────────────────────────────────────────────────────────

/// A channel item carrying pane output to attached clients.
pub type OutputItem = (PaneId, Vec<u8>);

/// Central store for all sessions.
pub struct SessionStore {
    sessions: HashMap<String, Session>,
    /// All output subscribers, keyed by session name.
    output_senders: HashMap<String, Vec<mpsc::UnboundedSender<OutputItem>>>,
    next_id: u64,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        SessionStore {
            sessions: HashMap::new(),
            output_senders: HashMap::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Return a receiver that will deliver pane output for the named session.
    pub fn subscribe_output(&mut self, session_name: &str) -> mpsc::UnboundedReceiver<OutputItem> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.output_senders
            .entry(session_name.to_string())
            .or_default()
            .push(tx);
        rx
    }

    /// Distribute pane output to all clients subscribed to `session_name`.
    ///
    /// This will be called by the PTY output-reading task once that is wired up.
    #[allow(dead_code)]
    fn broadcast_output(&mut self, session_name: &str, pane_id: PaneId, data: Vec<u8>) {
        if let Some(senders) = self.output_senders.get_mut(session_name) {
            senders.retain(|tx| tx.send((pane_id, data.clone())).is_ok());
        }
    }

    /// Ensure a session with `name` exists, creating it if necessary.
    pub fn ensure_session(&mut self, name: &str) {
        if !self.sessions.contains_key(name) {
            let pane = self
                .new_pane("pane 1", None)
                .expect("failed to create initial pane");
            let window_id = self.alloc_id();
            let window = Window {
                id: window_id,
                name: "window 1".to_string(),
                panes: vec![pane],
                active_pane_idx: 0,
            };
            let session = Session {
                name: name.to_string(),
                windows: vec![window],
                active_window_idx: 0,
            };
            self.sessions.insert(name.to_string(), session);
        }
    }

    pub fn session_info(&self, name: &str) -> SessionInfo {
        self.sessions[name].info()
    }

    fn new_pane(&mut self, name: &str, command: Option<String>) -> Result<Pane> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = if let Some(cmd) = command {
            cmd
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
        };

        let mut cmd = CommandBuilder::new(&shell);
        cmd.env("TERM", "xterm-256color");
        pair.slave.spawn_command(cmd)?;

        let writer = pair.master.take_writer()?;
        let id = self.alloc_id();
        Ok(Pane {
            id,
            name: name.to_string(),
            master: pair.master,
            writer,
        })
    }

    pub fn create_pane(
        &mut self,
        session_name: &str,
        window_id: WindowId,
        command: Option<String>,
    ) -> Result<()> {
        let session = self
            .sessions
            .get_mut(session_name)
            .ok_or_else(|| anyhow!("session not found: {session_name}"))?;
        let win_idx = session
            .find_window_idx_by_id(window_id)
            .ok_or_else(|| anyhow!("window {window_id} not found"))?;
        let pane_count = session.windows[win_idx].panes.len() + 1;
        let pane_name = format!("pane {pane_count}");
        let pane = self.new_pane(&pane_name, command)?;
        let session = self.sessions.get_mut(session_name).unwrap();
        session.windows[win_idx].panes.push(pane);
        Ok(())
    }

    pub fn send_input(&mut self, pane_id: PaneId, data: Vec<u8>) -> Result<()> {
        for session in self.sessions.values_mut() {
            for window in &mut session.windows {
                for pane in &mut window.panes {
                    if pane.id == pane_id {
                        pane.writer.write_all(&data)?;
                        return Ok(());
                    }
                }
            }
        }
        Err(anyhow!("pane {pane_id} not found"))
    }

    pub fn resize_pane(&mut self, pane_id: PaneId, cols: u16, rows: u16) -> Result<()> {
        for session in self.sessions.values_mut() {
            for window in &mut session.windows {
                for pane in &mut window.panes {
                    if pane.id == pane_id {
                        pane.master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        })?;
                        return Ok(());
                    }
                }
            }
        }
        Err(anyhow!("pane {pane_id} not found"))
    }

    pub fn navigate(&mut self, session_name: &str, direction: NavigateDirection) -> Result<()> {
        let session = self
            .sessions
            .get_mut(session_name)
            .ok_or_else(|| anyhow!("session not found"))?;

        match direction {
            NavigateDirection::Up | NavigateDirection::Down => {
                let win = session.active_window_mut();
                let idx = win.active_pane_idx;
                let new_idx = match direction {
                    NavigateDirection::Up => {
                        if idx == 0 {
                            return Err(anyhow!("no pane above"));
                        }
                        idx - 1
                    }
                    NavigateDirection::Down => {
                        if idx + 1 >= win.panes.len() {
                            return Err(anyhow!("no pane below"));
                        }
                        idx + 1
                    }
                    _ => unreachable!(),
                };
                win.active_pane_idx = new_idx;
            }
            NavigateDirection::Left | NavigateDirection::Right => {
                let cur_win_idx = session.active_window_idx;
                let new_win_idx = match direction {
                    NavigateDirection::Left => {
                        if cur_win_idx == 0 {
                            return Err(anyhow!("no window to the left"));
                        }
                        cur_win_idx - 1
                    }
                    NavigateDirection::Right => {
                        if cur_win_idx + 1 >= session.windows.len() {
                            return Err(anyhow!("no window to the right"));
                        }
                        cur_win_idx + 1
                    }
                    _ => unreachable!(),
                };
                session.active_window_idx = new_win_idx;
            }
        }
        Ok(())
    }

    pub fn move_pane(
        &mut self,
        session_name: &str,
        pane_id: PaneId,
        direction: MoveDirection,
    ) -> Result<()> {
        // Pre-allocate an ID in case we need to create a new window.
        let new_window_id = self.alloc_id();

        let session = self
            .sessions
            .get_mut(session_name)
            .ok_or_else(|| anyhow!("session not found"))?;

        let win_idx = session
            .find_window_idx_containing_pane(pane_id)
            .ok_or_else(|| anyhow!("pane {pane_id} not found"))?;

        match direction {
            MoveDirection::Up | MoveDirection::Down => {
                let win = &mut session.windows[win_idx];
                let pane_idx = win
                    .panes
                    .iter()
                    .position(|p| p.id == pane_id)
                    .ok_or_else(|| anyhow!("pane not found in window"))?;

                match direction {
                    MoveDirection::Up => {
                        if pane_idx == 0 {
                            return Err(anyhow!("pane is already at the top"));
                        }
                        win.panes.swap(pane_idx, pane_idx - 1);
                        if win.active_pane_idx == pane_idx {
                            win.active_pane_idx = pane_idx - 1;
                        } else if win.active_pane_idx == pane_idx - 1 {
                            win.active_pane_idx = pane_idx;
                        }
                    }
                    MoveDirection::Down => {
                        if pane_idx + 1 >= win.panes.len() {
                            return Err(anyhow!("pane is already at the bottom"));
                        }
                        win.panes.swap(pane_idx, pane_idx + 1);
                        if win.active_pane_idx == pane_idx {
                            win.active_pane_idx = pane_idx + 1;
                        } else if win.active_pane_idx == pane_idx + 1 {
                            win.active_pane_idx = pane_idx;
                        }
                    }
                    _ => unreachable!(),
                }
            }

            MoveDirection::Left | MoveDirection::Right => {
                let target_win_idx = match direction {
                    MoveDirection::Left => {
                        if win_idx == 0 {
                            // Will create a new window to the left.
                            None
                        } else {
                            Some(win_idx - 1)
                        }
                    }
                    MoveDirection::Right => {
                        if win_idx + 1 >= session.windows.len() {
                            // Will create a new window to the right.
                            None
                        } else {
                            Some(win_idx + 1)
                        }
                    }
                    _ => unreachable!(),
                };

                // Remove the pane from its current window.
                let win = &mut session.windows[win_idx];
                let pane_idx = win
                    .panes
                    .iter()
                    .position(|p| p.id == pane_id)
                    .ok_or_else(|| anyhow!("pane not found in window"))?;

                // Determine active state before removing the pane so we don't
                // index into an empty vec when the window loses its last pane.
                let was_active = win.active_pane_idx == pane_idx;

                let pane = win.panes.remove(pane_idx);

                // Adjust active pane index if necessary.
                if win.active_pane_idx >= win.panes.len() && !win.panes.is_empty() {
                    win.active_pane_idx = win.panes.len() - 1;
                }

                if let Some(dest_idx) = target_win_idx {
                    // Move to existing window.
                    session.windows[dest_idx].panes.push(pane);
                    if was_active {
                        let last = session.windows[dest_idx].panes.len() - 1;
                        session.windows[dest_idx].active_pane_idx = last;
                        session.active_window_idx = dest_idx;
                    }
                } else {
                    // Create a new window using the pre-allocated ID.
                    let new_win_idx = match direction {
                        MoveDirection::Left => win_idx,
                        MoveDirection::Right => win_idx + 1,
                        _ => unreachable!(),
                    };
                    let win_count = session.windows.len() + 1;
                    let new_window = Window {
                        id: new_window_id,
                        name: format!("window {win_count}"),
                        panes: vec![pane],
                        active_pane_idx: 0,
                    };
                    session.windows.insert(new_win_idx, new_window);
                    if was_active {
                        session.active_window_idx = new_win_idx;
                    }
                }

                // Remove the source window if it is now empty.
                let actual_win_idx = if target_win_idx.is_some() {
                    // No insert occurred, so indices are unchanged.
                    win_idx
                } else {
                    // An insert occurred; the original window shifted by one
                    // when we inserted to the left.
                    match direction {
                        MoveDirection::Left => win_idx + 1,
                        MoveDirection::Right => win_idx,
                        _ => unreachable!(),
                    }
                };

                if session.windows[actual_win_idx].panes.is_empty() {
                    session.windows.remove(actual_win_idx);
                    if session.active_window_idx >= session.windows.len() {
                        session.active_window_idx = session.windows.len().saturating_sub(1);
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shell used in tests so we don't rely on $SHELL being set.
    const TEST_SHELL: &str = "/bin/sh";

    // ─── Helpers ─────────────────────────────────────────────────────────────

    /// Create a fresh store that already contains a session with one window and one pane.
    fn store_with_session(name: &str) -> SessionStore {
        let mut store = SessionStore::new();
        store.ensure_session(name);
        store
    }

    /// Add an extra pane to the active window of `name`.
    fn add_pane(store: &mut SessionStore, session_name: &str) {
        let window_id = store.session_info(session_name).active_window_id;
        store
            .create_pane(session_name, window_id, Some(TEST_SHELL.to_string()))
            .expect("create_pane");
    }

    // ─── ensure_session ───────────────────────────────────────────────────────

    #[test]
    fn ensure_session_creates_one_window_one_pane() {
        let store = store_with_session("test");
        let info = store.session_info("test");

        assert_eq!(info.name, "test");
        assert_eq!(info.windows.len(), 1);
        assert_eq!(info.windows[0].panes.len(), 1);
        assert_eq!(info.active_window_id, info.windows[0].id);
        assert_eq!(
            info.windows[0].active_pane_id,
            info.windows[0].panes[0].id
        );
    }

    #[test]
    fn ensure_session_is_idempotent() {
        let mut store = store_with_session("test");
        store.ensure_session("test");
        let info = store.session_info("test");
        // Still exactly one window and one pane.
        assert_eq!(info.windows.len(), 1);
        assert_eq!(info.windows[0].panes.len(), 1);
    }

    #[test]
    fn ensure_session_multiple_independent_sessions() {
        let mut store = SessionStore::new();
        store.ensure_session("alpha");
        store.ensure_session("beta");

        let alpha = store.session_info("alpha");
        let beta = store.session_info("beta");
        assert_eq!(alpha.name, "alpha");
        assert_eq!(beta.name, "beta");
        // The two sessions must not share window or pane IDs.
        let alpha_win_id = alpha.windows[0].id;
        let beta_win_id = beta.windows[0].id;
        assert_ne!(alpha_win_id, beta_win_id);
        let alpha_pane_id = alpha.windows[0].panes[0].id;
        let beta_pane_id = beta.windows[0].panes[0].id;
        assert_ne!(alpha_pane_id, beta_pane_id);
    }

    // ─── create_pane ──────────────────────────────────────────────────────────

    #[test]
    fn create_pane_appends_pane_to_window() {
        let mut store = store_with_session("test");
        let window_id = store.session_info("test").active_window_id;

        store
            .create_pane("test", window_id, Some(TEST_SHELL.to_string()))
            .unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows[0].panes.len(), 2);
    }

    #[test]
    fn create_pane_assigns_unique_ids() {
        let mut store = store_with_session("test");
        let window_id = store.session_info("test").active_window_id;

        store
            .create_pane("test", window_id, Some(TEST_SHELL.to_string()))
            .unwrap();
        store
            .create_pane("test", window_id, Some(TEST_SHELL.to_string()))
            .unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows[0].panes.len(), 3);
        let ids: Vec<PaneId> = info.windows[0].panes.iter().map(|p| p.id).collect();
        let mut unique = ids.clone();
        unique.dedup();
        unique.sort();
        assert_eq!(unique.len(), ids.len(), "all pane IDs must be unique");
    }

    #[test]
    fn create_pane_unknown_session_returns_error() {
        let mut store = SessionStore::new();
        let err = store
            .create_pane("nonexistent", 1, None)
            .unwrap_err();
        assert!(
            err.to_string().contains("session not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn create_pane_unknown_window_returns_error() {
        let mut store = store_with_session("test");
        let err = store
            .create_pane("test", 9999, None)
            .unwrap_err();
        assert!(
            err.to_string().contains("window"),
            "unexpected error: {err}"
        );
    }

    // ─── navigate ─────────────────────────────────────────────────────────────

    #[test]
    fn navigate_down_moves_to_next_pane() {
        let mut store = store_with_session("test");
        let window_id = store.session_info("test").active_window_id;
        let first_pane_id = store.session_info("test").windows[0].panes[0].id;

        store
            .create_pane("test", window_id, Some(TEST_SHELL.to_string()))
            .unwrap();
        let second_pane_id = store.session_info("test").windows[0].panes[1].id;

        // Initially the first pane should be active.
        assert_eq!(
            store.session_info("test").windows[0].active_pane_id,
            first_pane_id
        );

        store
            .navigate("test", NavigateDirection::Down)
            .unwrap();

        assert_eq!(
            store.session_info("test").windows[0].active_pane_id,
            second_pane_id
        );
    }

    #[test]
    fn navigate_up_moves_to_previous_pane() {
        let mut store = store_with_session("test");
        let window_id = store.session_info("test").active_window_id;
        let first_pane_id = store.session_info("test").windows[0].panes[0].id;

        add_pane(&mut store, "test");

        store.navigate("test", NavigateDirection::Down).unwrap();
        store.navigate("test", NavigateDirection::Up).unwrap();

        assert_eq!(
            store.session_info("test").windows[0].active_pane_id,
            first_pane_id
        );
        let _ = window_id; // suppress unused warning
    }

    #[test]
    fn navigate_up_from_top_returns_error() {
        let mut store = store_with_session("test");
        let err = store
            .navigate("test", NavigateDirection::Up)
            .unwrap_err();
        assert!(
            err.to_string().contains("no pane above"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn navigate_down_from_bottom_returns_error() {
        let mut store = store_with_session("test");
        let err = store
            .navigate("test", NavigateDirection::Down)
            .unwrap_err();
        assert!(
            err.to_string().contains("no pane below"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn navigate_right_switches_active_window() {
        let mut store = store_with_session("test");
        let window1_id = store.session_info("test").active_window_id;

        // Create a second window by moving a new pane right.
        add_pane(&mut store, "test");
        let pane2_id = store.session_info("test").windows[0].panes[1].id;
        store
            .move_pane("test", pane2_id, MoveDirection::Right)
            .unwrap();

        let window2_id = store
            .session_info("test")
            .windows
            .iter()
            .find(|w| w.id != window1_id)
            .unwrap()
            .id;

        store.navigate("test", NavigateDirection::Right).unwrap();
        assert_eq!(store.session_info("test").active_window_id, window2_id);

        store.navigate("test", NavigateDirection::Left).unwrap();
        assert_eq!(store.session_info("test").active_window_id, window1_id);
    }

    #[test]
    fn navigate_left_from_leftmost_window_returns_error() {
        let mut store = store_with_session("test");
        let err = store
            .navigate("test", NavigateDirection::Left)
            .unwrap_err();
        assert!(
            err.to_string().contains("no window to the left"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn navigate_right_from_rightmost_window_returns_error() {
        let mut store = store_with_session("test");
        let err = store
            .navigate("test", NavigateDirection::Right)
            .unwrap_err();
        assert!(
            err.to_string().contains("no window to the right"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn navigate_unknown_session_returns_error() {
        let mut store = SessionStore::new();
        let err = store
            .navigate("ghost", NavigateDirection::Up)
            .unwrap_err();
        assert!(
            err.to_string().contains("session not found"),
            "unexpected: {err}"
        );
    }

    // ─── move_pane (vertical) ─────────────────────────────────────────────────

    #[test]
    fn move_pane_up_reorders_panes() {
        let mut store = store_with_session("test");
        let pane1_id = store.session_info("test").windows[0].panes[0].id;

        add_pane(&mut store, "test");
        let pane2_id = store.session_info("test").windows[0].panes[1].id;

        store.move_pane("test", pane2_id, MoveDirection::Up).unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows[0].panes[0].id, pane2_id);
        assert_eq!(info.windows[0].panes[1].id, pane1_id);
    }

    #[test]
    fn move_pane_down_reorders_panes() {
        let mut store = store_with_session("test");
        let pane1_id = store.session_info("test").windows[0].panes[0].id;

        add_pane(&mut store, "test");
        let pane2_id = store.session_info("test").windows[0].panes[1].id;

        store.move_pane("test", pane1_id, MoveDirection::Down).unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows[0].panes[0].id, pane2_id);
        assert_eq!(info.windows[0].panes[1].id, pane1_id);
    }

    #[test]
    fn move_pane_up_from_top_returns_error() {
        let mut store = store_with_session("test");
        let pane_id = store.session_info("test").windows[0].panes[0].id;

        let err = store
            .move_pane("test", pane_id, MoveDirection::Up)
            .unwrap_err();
        assert!(
            err.to_string().contains("already at the top"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn move_pane_down_from_bottom_returns_error() {
        let mut store = store_with_session("test");
        let pane_id = store.session_info("test").windows[0].panes[0].id;

        let err = store
            .move_pane("test", pane_id, MoveDirection::Down)
            .unwrap_err();
        assert!(
            err.to_string().contains("already at the bottom"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn move_pane_up_and_down_are_inverse() {
        let mut store = store_with_session("test");
        add_pane(&mut store, "test");
        add_pane(&mut store, "test");

        // Record original order.
        let original: Vec<PaneId> = store.session_info("test").windows[0]
            .panes
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(original.len(), 3);

        let pane_b = original[1];

        // Move middle pane up then back down.
        store.move_pane("test", pane_b, MoveDirection::Up).unwrap();
        store.move_pane("test", pane_b, MoveDirection::Down).unwrap();

        let after: Vec<PaneId> = store.session_info("test").windows[0]
            .panes
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(after, original);
    }

    // ─── move_pane (horizontal) ───────────────────────────────────────────────

    #[test]
    fn move_pane_right_creates_new_window_when_none_exists() {
        let mut store = store_with_session("test");
        let pane1_id = store.session_info("test").windows[0].panes[0].id;

        add_pane(&mut store, "test");
        let pane2_id = store.session_info("test").windows[0].panes[1].id;

        store
            .move_pane("test", pane2_id, MoveDirection::Right)
            .unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows.len(), 2);
        assert_eq!(info.windows[0].panes.len(), 1);
        assert_eq!(info.windows[0].panes[0].id, pane1_id);
        assert_eq!(info.windows[1].panes.len(), 1);
        assert_eq!(info.windows[1].panes[0].id, pane2_id);
    }

    #[test]
    fn move_pane_left_creates_new_window_to_the_left() {
        let mut store = store_with_session("test");
        let pane1_id = store.session_info("test").windows[0].panes[0].id;

        add_pane(&mut store, "test");
        let pane2_id = store.session_info("test").windows[0].panes[1].id;

        store
            .move_pane("test", pane2_id, MoveDirection::Left)
            .unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows.len(), 2);
        // The new window is to the LEFT, so it should be at index 0.
        assert_eq!(info.windows[0].panes[0].id, pane2_id);
        assert_eq!(info.windows[1].panes[0].id, pane1_id);
    }

    #[test]
    fn move_pane_right_to_existing_window() {
        let mut store = store_with_session("test");
        let window1_id = store.session_info("test").active_window_id;

        // Build window 2 by moving a new pane right.
        add_pane(&mut store, "test");
        let pane2_id = store.session_info("test").windows[0].panes[1].id;
        store
            .move_pane("test", pane2_id, MoveDirection::Right)
            .unwrap();

        let window2_id = store
            .session_info("test")
            .windows
            .iter()
            .find(|w| w.id != window1_id)
            .unwrap()
            .id;

        // Now create another pane in window 1 and move it right.
        store
            .create_pane("test", window1_id, Some(TEST_SHELL.to_string()))
            .unwrap();
        let new_pane_id = store
            .session_info("test")
            .windows
            .iter()
            .find(|w| w.id == window1_id)
            .unwrap()
            .panes
            .last()
            .unwrap()
            .id;

        store
            .move_pane("test", new_pane_id, MoveDirection::Right)
            .unwrap();

        let info = store.session_info("test");
        let win2 = info.windows.iter().find(|w| w.id == window2_id).unwrap();
        assert_eq!(win2.panes.len(), 2);
        assert!(win2.panes.iter().any(|p| p.id == new_pane_id));
    }

    #[test]
    fn move_pane_removes_source_window_when_it_becomes_empty() {
        let mut store = store_with_session("test");
        let window1_id = store.session_info("test").active_window_id;
        let pane1_id = store.session_info("test").windows[0].panes[0].id;

        // Move the only pane right; the source window must be removed.
        store
            .move_pane("test", pane1_id, MoveDirection::Right)
            .unwrap();

        let info = store.session_info("test");
        assert_eq!(info.windows.len(), 1, "original empty window should be removed");
        assert!(
            !info.windows.iter().any(|w| w.id == window1_id),
            "original window should no longer exist"
        );
        // The pane should now live in the remaining window.
        assert_eq!(info.windows[0].panes[0].id, pane1_id);
    }

    #[test]
    fn move_pane_unknown_pane_returns_error() {
        let mut store = store_with_session("test");
        let err = store
            .move_pane("test", 9999, MoveDirection::Up)
            .unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn move_pane_unknown_session_returns_error() {
        let mut store = SessionStore::new();
        let err = store
            .move_pane("ghost", 1, MoveDirection::Up)
            .unwrap_err();
        assert!(
            err.to_string().contains("session not found"),
            "unexpected: {err}"
        );
    }
}
