use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// minmux — a minimalist terminal multiplexer.
#[derive(Debug, Parser)]
#[command(name = "minmux", version, about)]
pub struct Options {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Attach to a session (default).  Starts a new session server if none is running.
    Attach {
        /// Name of the session to attach to (or create).
        #[arg(default_value = "main")]
        session: String,
    },

    /// Run the session server on the given Unix socket (internal use).
    #[command(hide = true)]
    Daemon {
        /// Path to the Unix socket the server should listen on.
        socket: PathBuf,
    },
}

/// Entry point called from `main`.
pub fn run() -> Result<()> {
    let opts = Options::parse();
    let command = opts.command.unwrap_or(Command::Attach {
        session: "main".to_string(),
    });

    match command {
        Command::Attach { session } => attach(&session),
        Command::Daemon { socket } => run_daemon(&socket),
    }
}

fn attach(session_name: &str) -> Result<()> {
    let socket_path = default_socket_path()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        // If the socket is not present, spawn a daemon and wait for it.
        if !socket_path.exists() {
            start_daemon(&socket_path).await?;

            // Wait up to one second for the daemon to create its socket.
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if socket_path.exists() {
                    break;
                }
            }
        }

        minmux_client::run(&socket_path, session_name).await
    })
}

fn run_daemon(socket_path: &PathBuf) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(minmux_daemon::run(socket_path))
}

/// Returns the path to the session server's Unix socket.
fn default_socket_path() -> Result<PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_directory().join(".local").join("run"));
    Ok(runtime_dir.join("minmux").join("minmux.sock"))
}

fn home_directory() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Spawn the session server as a detached background process.
async fn start_daemon(socket_path: &PathBuf) -> Result<()> {
    let exe = std::env::current_exe().context("could not determine current executable")?;
    tokio::process::Command::new(&exe)
        .arg("daemon")
        .arg(socket_path)
        .spawn()
        .context("failed to spawn daemon process")?;
    Ok(())
}
