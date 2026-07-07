pub mod app;
pub mod input;
pub mod ui;

use anyhow::Result;
use std::path::Path;

/// Launch the interactive client, connecting to the daemon at `socket_path`
/// and attaching to `session_name`.
pub async fn run(socket_path: &Path, session_name: &str) -> Result<()> {
    app::run(socket_path, session_name).await
}
