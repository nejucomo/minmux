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

    fn active_pane_id(&self) -> PaneId {
        self.panes[self.active_pane_idx].id
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

    #[allow(dead_code)]
    fn active_window(&self) -> &Window {
        &self.windows[self.active_window_idx]
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
                let pane = win.panes.remove(pane_idx);

                // Adjust active pane index if necessary.
                if win.active_pane_idx >= win.panes.len() && !win.panes.is_empty() {
                    win.active_pane_idx = win.panes.len() - 1;
                }

                let was_active = pane.id == session.windows[win_idx].active_pane_id();

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
