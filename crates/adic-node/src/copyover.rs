use anyhow::{anyhow, Result};
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nix::unistd::{execve, pipe};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd as AsRawFdTrait;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::Path;
use tracing::{debug, error, info, warn};

/// State to be preserved across copyover
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyoverState {
    /// File descriptor for the HTTP API listener
    pub api_listener_fd: Option<i32>,

    /// Current configuration path
    pub config_path: String,

    /// Data directory path
    pub data_dir: String,

    /// Peer information to restore
    pub peers: Vec<PeerState>,

    /// Current version
    pub version: String,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerState {
    pub peer_id: String,
    pub addresses: Vec<String>,
    pub reputation: f32,
    pub last_seen: i64,
}

/// Manages the copyover process for graceful binary replacement
pub struct CopyoverManager {
    /// Path to the new binary
    #[allow(dead_code)]
    new_binary: Option<String>,

    /// Current state to preserve
    state: Option<CopyoverState>,
}

impl Default for CopyoverManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyoverManager {
    /// Create a new copyover manager
    pub fn new() -> Self {
        Self {
            new_binary: None,
            state: None,
        }
    }

    /// Prepare for copyover by collecting state
    pub fn prepare_state(
        &mut self,
        api_fd: Option<RawFd>,
        config_path: String,
        data_dir: String,
        version: String,
    ) -> Result<()> {
        info!("📦 Preparing copyover state");

        // Clear CLOEXEC flag on API listener if present
        if let Some(fd) = api_fd {
            self.clear_cloexec(fd)?;
        }

        self.state = Some(CopyoverState {
            api_listener_fd: api_fd,
            config_path,
            data_dir,
            peers: Vec::new(), // Would be populated from PeerManager
            version,
            metadata: HashMap::new(),
        });

        Ok(())
    }

    /// Clear the CLOEXEC flag on a file descriptor
    fn clear_cloexec(&self, fd: RawFd) -> Result<()> {
        let flags = fcntl(fd, FcntlArg::F_GETFD)?;
        let mut new_flags = FdFlag::from_bits_truncate(flags);
        new_flags.remove(FdFlag::FD_CLOEXEC);
        fcntl(fd, FcntlArg::F_SETFD(new_flags))?;

        debug!(fd = fd, "🔓 Cleared CLOEXEC flag on file descriptor");
        Ok(())
    }

    /// Execute copyover to new binary
    pub fn execute_copyover(&self, new_binary_path: &str) -> Result<()> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| anyhow!("No state prepared for copyover"))?;

        info!(
            new_binary = %new_binary_path,
            "🔄 Executing copyover"
        );

        // Create pipe for state transfer
        let (read_fd, write_fd) = pipe()?;

        // Serialize state to JSON
        let state_json = serde_json::to_string(state)?;

        // Fork process
        match unsafe { nix::unistd::fork() } {
            Ok(nix::unistd::ForkResult::Parent { child }) => {
                // Parent process
                drop(write_fd); // Close write end of pipe in parent

                // Prepare arguments for new binary
                let read_fd_raw = read_fd.as_raw_fd();
                let mut args = vec![
                    CString::new(new_binary_path)?,
                    CString::new("--copyover-recovery")?,
                    CString::new(format!("--copyover-pipe={}", read_fd_raw))?,
                ];

                // Add API listener FD if present
                if let Some(api_fd) = state.api_listener_fd {
                    args.push(CString::new(format!("--copyover-fd={}", api_fd))?);
                }

                // Add original arguments
                args.push(CString::new("start")?);
                args.push(CString::new("--data-dir")?);
                args.push(CString::new(state.data_dir.as_str())?);

                // Prepare environment
                let env: Vec<CString> = env::vars()
                    .map(|(k, v)| CString::new(format!("{}={}", k, v)))
                    .collect::<Result<Vec<_>, _>>()?;

                info!(
                    child_pid = ?child,
                    "🚀 Launching new binary via exec"
                );

                // Replace current process with new binary
                execve(&CString::new(new_binary_path)?, &args, &env)?;

                // If we get here, exec failed
                Err(anyhow!("execve failed"))
            }
            Ok(nix::unistd::ForkResult::Child) => {
                // Child process - write state and exit
                drop(read_fd); // Close read end of pipe in child

                // Write state to pipe
                let write_fd_raw = write_fd.as_raw_fd();
                let mut pipe_writer = unsafe { fs::File::from_raw_fd(write_fd_raw) };
                pipe_writer.write_all(state_json.as_bytes())?;
                pipe_writer.sync_all()?;
                std::mem::forget(pipe_writer); // Don't close the fd on drop

                debug!("📝 State written to pipe, child exiting");
                std::process::exit(0);
            }
            Err(e) => {
                error!(
                    error = %e,
                    "❌ Fork failed"
                );
                Err(anyhow!("Fork failed: {}", e))
            }
        }
    }

    /// Recover from copyover (called in new process)
    #[allow(dead_code)]
    pub fn recover_from_copyover(pipe_fd: RawFd) -> Result<CopyoverState> {
        info!(pipe_fd = pipe_fd, "📥 Recovering from copyover");

        // Read state from pipe
        let mut pipe_reader = unsafe { fs::File::from_raw_fd(pipe_fd) };
        let mut state_json = String::new();
        pipe_reader.read_to_string(&mut state_json)?;

        // Deserialize state
        let state: CopyoverState = serde_json::from_str(&state_json)?;

        info!(
            version = %state.version,
            api_fd = ?state.api_listener_fd,
            peers = state.peers.len(),
            "✅ Successfully recovered copyover state"
        );

        Ok(state)
    }

    /// Perform a safe copyover with rollback on failure
    pub async fn safe_copyover(&self, new_binary: &str) -> Result<()> {
        // Backup current binary
        let current_binary = env::current_exe()?;
        let backup_path = format!("{}.backup", current_binary.display());
        fs::copy(&current_binary, &backup_path)?;

        info!(
            backup = %backup_path,
            "💾 Created backup of current binary"
        );

        // Perform health check on new binary
        if !self.verify_new_binary(new_binary).await? {
            return Err(anyhow!("New binary failed health check"));
        }

        // Execute copyover
        if let Err(e) = self.execute_copyover(new_binary) {
            error!(
                error = %e,
                "❌ Copyover failed, restoring backup"
            );

            // Restore backup
            fs::copy(&backup_path, current_binary)?;
            return Err(e);
        }

        Ok(())
    }

    /// Verify new binary is valid
    async fn verify_new_binary(&self, binary_path: &str) -> Result<bool> {
        use std::process::Command;

        info!(
            binary = %binary_path,
            "🔍 Verifying new binary"
        );

        // Check if binary exists and is executable
        if !Path::new(binary_path).exists() {
            return Ok(false);
        }

        // Try running with --version to verify it's valid
        let output = Command::new(binary_path).arg("--version").output()?;

        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout);
            info!(
                version = %version.trim(),
                "✅ New binary verified"
            );
            Ok(true)
        } else {
            warn!("⚠️ New binary failed verification");
            Ok(false)
        }
    }
}

/// Helper to reconstruct TcpListener from file descriptor
#[cfg(unix)]
#[allow(dead_code)]
pub fn tcp_listener_from_fd(fd: RawFd) -> Result<tokio::net::TcpListener> {
    use std::net::TcpListener as StdTcpListener;

    // Reconstruct std TcpListener from raw fd
    let std_listener = unsafe { StdTcpListener::from_raw_fd(fd) };

    // Convert to tokio TcpListener
    std_listener.set_nonblocking(true)?;
    let tokio_listener = tokio::net::TcpListener::from_std(std_listener)?;

    Ok(tokio_listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copyover_state_serialization() {
        let state = CopyoverState {
            api_listener_fd: Some(3),
            config_path: "/path/to/config.toml".to_string(),
            data_dir: "/data".to_string(),
            peers: vec![PeerState {
                peer_id: "peer1".to_string(),
                addresses: vec!["127.0.0.1:9000".to_string()],
                reputation: 75.0,
                last_seen: 1234567890,
            }],
            version: "0.1.0".to_string(),
            metadata: HashMap::new(),
        };

        let json = serde_json::to_string(&state).unwrap();
        let recovered: CopyoverState = serde_json::from_str(&json).unwrap();

        assert_eq!(recovered.api_listener_fd, Some(3));
        assert_eq!(recovered.config_path, "/path/to/config.toml");
        assert_eq!(recovered.peers.len(), 1);
        assert_eq!(recovered.peers[0].peer_id, "peer1");
    }
}
