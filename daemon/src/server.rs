use anyhow::Result;
use std::{path::Path, sync::Arc};
use tokio::{net::UnixListener, sync::Mutex};

use crate::session::SessionStore;
use minmux_ipc::{ClientMessage, Connection, ServerMessage};

/// Run the Unix-socket server until the task is cancelled.
pub async fn run(socket_path: &Path) -> Result<()> {
    // Remove stale socket if it exists.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    // Ensure parent directory exists.
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    let store: Arc<Mutex<SessionStore>> = Arc::new(Mutex::new(SessionStore::new()));

    loop {
        let (stream, _addr) = listener.accept().await?;
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, store).await {
                eprintln!("[daemon] client error: {e}");
            }
        });
    }
}

async fn handle_client(
    stream: tokio::net::UnixStream,
    store: Arc<Mutex<SessionStore>>,
) -> Result<()> {
    let conn = Connection::new(stream);
    // The first message must be Attach.
    let first: ClientMessage = conn.recv().await?;
    let session_name = match first {
        ClientMessage::Attach { session_name } => session_name,
        other => {
            conn.send(&ServerMessage::Error {
                message: format!("expected Attach, got {other:?}"),
            })
            .await?;
            return Ok(());
        }
    };

    // Ensure the session exists.
    {
        let mut store = store.lock().await;
        store.ensure_session(&session_name);
    }

    // Send initial state.
    {
        let store = store.lock().await;
        let info = store.session_info(&session_name);
        conn.send(&ServerMessage::SessionState(info)).await?;
    }

    // Subscribe to pane output for this session.
    let output_rx = {
        let mut store = store.lock().await;
        store.subscribe_output(&session_name)
    };

    // Spawn a task to forward pane output to the client.
    let conn = Arc::new(conn);
    let conn_output = Arc::clone(&conn);
    let output_task = tokio::spawn(async move {
        let mut rx = output_rx;
        while let Some((pane_id, data)) = rx.recv().await {
            if conn_output
                .send(&ServerMessage::PaneOutput { pane_id, data })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Main message loop.
    loop {
        let msg: ClientMessage = match conn.recv().await {
            Ok(m) => m,
            Err(_) => break,
        };

        match msg {
            ClientMessage::Attach { .. } => {
                conn.send(&ServerMessage::Error {
                    message: "already attached".to_string(),
                })
                .await?;
            }

            ClientMessage::CreatePane { window_id, command } => {
                let result = {
                    let mut store = store.lock().await;
                    store.create_pane(&session_name, window_id, command)
                };
                match result {
                    Ok(()) => {
                        let store = store.lock().await;
                        let info = store.session_info(&session_name);
                        conn.send(&ServerMessage::SessionState(info)).await?;
                    }
                    Err(e) => {
                        conn.send(&ServerMessage::Error {
                            message: e.to_string(),
                        })
                        .await?;
                    }
                }
            }

            ClientMessage::MovePane { pane_id, direction } => {
                let result = {
                    let mut store = store.lock().await;
                    store.move_pane(&session_name, pane_id, direction)
                };
                match result {
                    Ok(()) => {
                        let store = store.lock().await;
                        let info = store.session_info(&session_name);
                        conn.send(&ServerMessage::SessionState(info)).await?;
                    }
                    Err(e) => {
                        conn.send(&ServerMessage::Error {
                            message: e.to_string(),
                        })
                        .await?;
                    }
                }
            }

            ClientMessage::Navigate {
                session_name: _,
                direction,
            } => {
                let result = {
                    let mut store = store.lock().await;
                    store.navigate(&session_name, direction)
                };
                match result {
                    Ok(()) => {
                        let store = store.lock().await;
                        let info = store.session_info(&session_name);
                        conn.send(&ServerMessage::SessionState(info)).await?;
                    }
                    Err(e) => {
                        conn.send(&ServerMessage::Error {
                            message: e.to_string(),
                        })
                        .await?;
                    }
                }
            }

            ClientMessage::SendInput { pane_id, data } => {
                let result = {
                    let mut store = store.lock().await;
                    store.send_input(pane_id, data)
                };
                if let Err(e) = result {
                    conn.send(&ServerMessage::Error {
                        message: e.to_string(),
                    })
                    .await?;
                }
            }

            ClientMessage::ResizePane { pane_id, cols, rows } => {
                let result = {
                    let mut store = store.lock().await;
                    store.resize_pane(pane_id, cols, rows)
                };
                if let Err(e) = result {
                    conn.send(&ServerMessage::Error {
                        message: e.to_string(),
                    })
                    .await?;
                }
            }

            ClientMessage::Detach => {
                conn.send(&ServerMessage::Detached).await?;
                break;
            }
        }
    }

    output_task.abort();
    Ok(())
}
