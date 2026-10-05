//! Local control socket: the `screensight` CLI talks to the daemon over a
//! Unix socket carrying one JSON request per line and one JSON response back.
//!
//! The socket is normally created by systemd (`screensight.socket`) and handed
//! to the daemon as file descriptor 3 via socket activation. When that is not
//! present (development, headless runs) the daemon binds
//! [`DEFAULT_SOCKET_PATH`] itself.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::{UnixListener, UnixStream as TokioUnixStream};

use crate::protocol::{ControlRequest, ControlResponse};
use crate::runtime::Runtime;

/// Where the control socket lives when the daemon binds it itself.
pub const DEFAULT_SOCKET_PATH: &str = "/run/screensight/control.sock";

/// Take the socket-activation listener if systemd provided one (fd 3).
#[must_use]
pub fn systemd_listener() -> Option<std::os::unix::net::UnixListener> {
    listenfd::ListenFd::from_env()
        .take_unix_listener(0)
        .ok()
        .flatten()
}

/// Bind the control socket ourselves, removing a stale socket first.
pub fn bind_socket(path: &Path) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing stale socket {}", path.display()))?;
    }
    let std_listener = std::os::unix::net::UnixListener::bind(path)
        .with_context(|| format!("binding {}", path.display()))?;
    std_listener
        .set_nonblocking(true)
        .context("setting control socket non-blocking")?;
    UnixListener::from_std(std_listener).context("adopting control socket")
}

/// Adopt an already-open listener (e.g. from systemd socket activation).
pub fn from_std(listener: std::os::unix::net::UnixListener) -> Result<UnixListener> {
    listener
        .set_nonblocking(true)
        .context("setting control socket non-blocking")?;
    UnixListener::from_std(listener).context("adopting control socket")
}

/// Serve control requests until the process exits.
pub async fn serve(runtime: Arc<Runtime>, listener: UnixListener) -> Result<()> {
    loop {
        let (stream, _addr) = listener
            .accept()
            .await
            .context("accepting control connection")?;
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(runtime, stream).await {
                log::debug!("control connection ended: {err:#}");
            }
        });
    }
}

async fn handle_connection(runtime: Arc<Runtime>, stream: TokioUnixStream) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = TokioBufReader::new(read_half).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<ControlRequest>(&line) {
            Ok(request) => handle_request(&runtime, request),
            Err(err) => ControlResponse::error(format!("bad request: {err}")),
        };
        let mut encoded = serde_json::to_vec(&response).context("encoding control response")?;
        encoded.push(b'\n');
        write_half.write_all(&encoded).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Apply a control request to the runtime.
fn handle_request(runtime: &Runtime, request: ControlRequest) -> ControlResponse {
    match request {
        ControlRequest::Status => ControlResponse::status(runtime.status()),
        ControlRequest::Pair => match runtime.arm_pairing() {
            Ok(_) => ControlResponse::status(runtime.status()),
            Err(err) => ControlResponse::error(format!("{err:#}")),
        },
        ControlRequest::CancelPair => {
            runtime.cancel_pairing();
            ControlResponse::ok()
        }
        ControlRequest::Confirm => match runtime.confirm_pairing() {
            Ok(Some(instance)) => {
                log::info!(
                    "pairing confirmed via control socket for {}",
                    instance.ha_id
                );
                ControlResponse::status(runtime.status())
            }
            Ok(None) => ControlResponse::error("no pending pairing to confirm"),
            Err(err) => ControlResponse::error(format!("{err:#}")),
        },
        ControlRequest::Reject => match runtime.reject_pairing() {
            Ok(()) => ControlResponse::status(runtime.status()),
            Err(err) => ControlResponse::error(format!("{err:#}")),
        },
        ControlRequest::Unpair { id, all } => {
            if !all && id.is_none() {
                return ControlResponse::error("unpair requires an id or --all");
            }
            match runtime.unpair(id.as_deref(), all) {
                Ok(0) => ControlResponse::error("no matching paired instance"),
                Ok(_) => ControlResponse::status(runtime.status()),
                Err(err) => ControlResponse::error(format!("{err:#}")),
            }
        }
        ControlRequest::Select { id } => match runtime.select(&id) {
            Ok(true) => ControlResponse::status(runtime.status()),
            Ok(false) => ControlResponse::error(format!("no paired instance with id {id}")),
            Err(err) => ControlResponse::error(format!("{err:#}")),
        },
    }
}

/// Send one control request from the CLI (blocking) and read the response.
pub fn request(socket: &Path, request: &ControlRequest) -> Result<ControlResponse> {
    let mut stream = UnixStream::connect(socket).with_context(|| {
        format!(
            "connecting to {} (is screensightd running?)",
            socket.display()
        )
    })?;
    let mut encoded = serde_json::to_vec(request).context("encoding control request")?;
    encoded.push(b'\n');
    stream
        .write_all(&encoded)
        .context("sending control request")?;
    stream.flush().context("flushing control request")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("reading control response")?;
    serde_json::from_str(&line).context("parsing control response")
}

/// Resolve the control socket path, honouring `SCREENSIGHT_CONTROL_SOCKET`.
#[must_use]
pub fn socket_path() -> PathBuf {
    std::env::var_os("SCREENSIGHT_CONTROL_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}
