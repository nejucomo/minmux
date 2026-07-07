use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream,
    },
    sync::Mutex,
};

/// A framed JSON connection over a Unix socket.
///
/// Each message is a single JSON value followed by a newline (`\n`).
pub struct Connection {
    reader: Mutex<BufReader<OwnedReadHalf>>,
    writer: Mutex<OwnedWriteHalf>,
}

impl Connection {
    /// Wrap an existing [`UnixStream`].
    pub fn new(stream: UnixStream) -> Self {
        let (read_half, write_half) = stream.into_split();
        Connection {
            reader: Mutex::new(BufReader::new(read_half)),
            writer: Mutex::new(write_half),
        }
    }

    /// Connect to the Unix socket at `path`.
    pub async fn connect(path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(path).await?;
        Ok(Self::new(stream))
    }

    /// Send a message by serialising it to JSON and writing a newline-terminated line.
    pub async fn send<M: Serialize>(&self, msg: &M) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(line.as_bytes()).await?;
        Ok(())
    }

    /// Receive one message by reading a newline-terminated JSON line.
    pub async fn recv<M: DeserializeOwned>(&self) -> Result<M> {
        let mut line = String::new();
        let mut reader = self.reader.lock().await;
        reader.read_line(&mut line).await?;
        if line.is_empty() {
            return Err(anyhow::anyhow!("connection closed"));
        }
        let msg = serde_json::from_str(&line)?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMessage, ServerMessage};

    // Helper: create a connected in-process socket pair.
    fn socket_pair() -> (tokio::net::UnixStream, tokio::net::UnixStream) {
        tokio::net::UnixStream::pair().expect("UnixStream::pair")
    }

    // ─── Happy-path ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn send_recv_single_message_roundtrip() {
        let (a, b) = socket_pair();
        let sender = Connection::new(a);
        let receiver = Connection::new(b);

        let msg = ClientMessage::Attach {
            session_name: "roundtrip-session".to_string(),
        };
        sender.send(&msg).await.unwrap();

        let received: ClientMessage = receiver.recv().await.unwrap();
        match received {
            ClientMessage::Attach { session_name } => {
                assert_eq!(session_name, "roundtrip-session")
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_recv_multiple_messages_in_order() {
        let (a, b) = socket_pair();
        let sender = Connection::new(a);
        let receiver = Connection::new(b);

        let messages = vec![
            ServerMessage::Error {
                message: "first".to_string(),
            },
            ServerMessage::Detached,
            ServerMessage::Error {
                message: "third".to_string(),
            },
        ];

        for m in &messages {
            sender.send(m).await.unwrap();
        }

        let m1: ServerMessage = receiver.recv().await.unwrap();
        assert!(matches!(m1, ServerMessage::Error { .. }));

        let m2: ServerMessage = receiver.recv().await.unwrap();
        assert!(matches!(m2, ServerMessage::Detached));

        let m3: ServerMessage = receiver.recv().await.unwrap();
        match m3 {
            ServerMessage::Error { message } => assert_eq!(message, "third"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_recv_binary_payload_roundtrip() {
        let (a, b) = socket_pair();
        let sender = Connection::new(a);
        let receiver = Connection::new(b);

        let data: Vec<u8> = (0u8..=255).collect();
        let msg = ServerMessage::PaneOutput {
            pane_id: 7,
            data: data.clone(),
        };
        sender.send(&msg).await.unwrap();

        let back: ServerMessage = receiver.recv().await.unwrap();
        match back {
            ServerMessage::PaneOutput { pane_id, data: back_data } => {
                assert_eq!(pane_id, 7);
                assert_eq!(back_data, data);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_works_concurrently_from_both_sides() {
        let (a, b) = socket_pair();
        let conn_a = Connection::new(a);
        let conn_b = Connection::new(b);

        // Both sides send simultaneously; each receives from the other.
        let send_a = conn_a.send(&ClientMessage::Detach);
        let send_b = conn_b.send(&ServerMessage::Detached);
        tokio::try_join!(send_a, send_b).unwrap();

        let from_b: ServerMessage = conn_a.recv().await.unwrap();
        assert!(matches!(from_b, ServerMessage::Detached));

        let from_a: ClientMessage = conn_b.recv().await.unwrap();
        assert!(matches!(from_a, ClientMessage::Detach));
    }

    // ─── Error cases ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn recv_on_closed_connection_returns_error() {
        let (a, b) = socket_pair();
        let receiver = Connection::new(b);

        // Close the sending end without writing anything.
        drop(a);

        let result = receiver.recv::<ClientMessage>().await;
        assert!(result.is_err(), "expected error on closed connection");
    }

    #[tokio::test]
    async fn recv_invalid_json_returns_error() {
        let (mut a, b) = socket_pair();
        let receiver = Connection::new(b);

        // Write a line that is not valid JSON.
        a.write_all(b"this is not json\n").await.unwrap();
        a.flush().await.unwrap();

        let result = receiver.recv::<ClientMessage>().await;
        assert!(result.is_err(), "expected JSON parse error");
    }

    #[tokio::test]
    async fn recv_empty_json_object_for_wrong_type_returns_error() {
        let (mut a, b) = socket_pair();
        let receiver = Connection::new(b);

        // `{}` is valid JSON but not a valid ClientMessage.
        a.write_all(b"{}\n").await.unwrap();
        a.flush().await.unwrap();

        let result = receiver.recv::<ClientMessage>().await;
        assert!(result.is_err(), "expected variant parse error");
    }
}
