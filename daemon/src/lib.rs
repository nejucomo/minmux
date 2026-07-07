pub mod server;
pub mod session;

use anyhow::Result;
use std::path::Path;

/// Start the daemon, listening on the given Unix socket path.
pub async fn run(socket_path: &Path) -> Result<()> {
    server::run(socket_path).await
}
