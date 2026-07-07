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
