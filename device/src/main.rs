//! `screensightd` — the Screensight device daemon.
//!
//! Owns the identity and persisted pairings, the local control socket, the
//! Home Assistant WebSocket link, the mDNS advertisement and (with the `gui`
//! feature) the 800×480 panel renderer.
//!
//! Runs headless on any machine for development and CI; the physical device
//! build additionally enables the `gui` feature and paints under `cage`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;

use screensight::control;
use screensight::runtime::{Runtime, Screen};
use screensight::store;

#[derive(Parser, Debug)]
#[command(name = "screensightd", version, about = "Screensight display daemon")]
struct Args {
    /// Run without the GPUI panel renderer (always the case when built without
    /// the `gui` feature).
    #[arg(long)]
    headless: bool,

    /// Directory for `state.json`.
    #[arg(long, env = "SCREENSIGHT_STATE_DIR")]
    state_dir: Option<PathBuf>,

    /// Unix control socket path.
    #[arg(long, env = "SCREENSIGHT_CONTROL_SOCKET")]
    control_socket: Option<PathBuf>,

    /// TCP port for the Home Assistant WebSocket.
    #[arg(long, env = "SCREENSIGHT_HTTP_PORT", default_value_t = 8765)]
    port: u16,

    /// Human-facing model name advertised over mDNS.
    #[arg(long, env = "SCREENSIGHT_MODEL", default_value = "Screensight Studio")]
    model: String,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,screensight=info,screensightd=info"),
    )
    .init();
    let args = Args::parse();
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    tokio.block_on(run(args))
}

async fn run(args: Args) -> Result<()> {
    let state_dir = args.state_dir.unwrap_or_else(store::state_dir);
    let runtime = Arc::new(Runtime::load(
        state_dir.clone(),
        &args.model,
        env!("CARGO_PKG_VERSION"),
    )?);
    log::info!(
        "screensightd {} starting (state {}, model {})",
        env!("CARGO_PKG_VERSION"),
        state_dir.display(),
        args.model,
    );

    // First boot with nothing paired: open a pairing window so the panel shows
    // a code straight away.
    if !runtime.status().pairing && runtime.status().instances.is_empty() {
        if let Err(err) = runtime.arm_pairing() {
            log::warn!("could not open initial pairing window: {err:#}");
        }
    }

    // Control socket: systemd hands us fd 3, otherwise bind it ourselves.
    let socket_path = args.control_socket.unwrap_or_else(control::socket_path);
    let listener = match control::systemd_listener() {
        Some(std_listener) => {
            log::info!("using systemd-activated control socket");
            control::from_std(std_listener)?
        }
        None => {
            log::info!("binding control socket at {}", socket_path.display());
            control::bind_socket(&socket_path)?
        }
    };
    let control_runtime = Arc::clone(&runtime);
    tokio::spawn(async move {
        if let Err(err) = control::serve(control_runtime, listener).await {
            log::error!("control socket server stopped: {err:#}");
        }
    });

    // WebSocket server for the Home Assistant link.
    let ws_runtime = Arc::clone(&runtime);
    let port = args.port;
    tokio::spawn(async move {
        if let Err(err) = screensight::server::serve(ws_runtime, port).await {
            log::error!("websocket server stopped: {err:#}");
        }
    });

    // mDNS advertisement (best-effort: the daemon runs fine without Avahi).
    let mdns_runtime = Arc::clone(&runtime);
    tokio::spawn(async move {
        if let Err(err) = screensight::mdns::advertise(mdns_runtime, port).await {
            log::warn!("mDNS advertisement unavailable: {err:#}");
        }
    });

    // Renderer / panel: with the `gui` feature this paints under cage;
    // otherwise we log screen transitions so headless runs remain observable.
    run_panel(runtime, args.model, args.headless).await;

    Ok(())
}

/// Drive the panel. On the device this blocks on the GPUI renderer (feeding it
/// screen changes); headless it watches the runtime and logs transitions.
async fn run_panel(runtime: Arc<Runtime>, model: String, headless: bool) {
    #[cfg(feature = "gui")]
    {
        if !headless {
            log::info!("starting GPUI panel");
            // GPUI blocks the main thread, so keep advancing the runtime's
            // time-based state and feeding the panel from a worker task.
            tokio::spawn(run_headless(Arc::clone(&runtime)));
            if let Err(err) = screensight::panel::run(runtime, model) {
                log::error!("panel stopped: {err:#}");
            }
            return;
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        // Without the `gui` feature the daemon is always headless.
        let _ = (model, headless);
    }
    run_headless(runtime).await;
}

/// Headless fallback: watch the runtime and log screen transitions. With the
/// `gui` feature enabled this path still feeds the panel when running headless.
async fn run_headless(runtime: Arc<Runtime>) {
    let mut last: Option<Screen> = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    loop {
        ticker.tick().await;
        let screen = runtime.tick(Instant::now());
        if last.as_ref() != Some(&screen) {
            log::info!("panel screen -> {}", describe(&screen));
            #[cfg(feature = "gui")]
            screensight::panel::set_screen(screen.clone());
            last = Some(screen);
        }
    }
}

/// One-line description of a [`Screen`] for logs.
fn describe(screen: &Screen) -> String {
    match screen {
        Screen::Unpaired => "unpaired".to_owned(),
        Screen::NoHome => "no home discovered".to_owned(),
        Screen::Timeout => "pairing window timed out".to_owned(),
        Screen::Pairing { code } => format!("pairing (code {code})"),
        Screen::Confirm { ha_name, .. } => format!("confirm {ha_name}"),
        Screen::Display { text: Some(text) } => format!("display {:?}", truncate(text, 40)),
        Screen::Display { text: None } => "display (no text)".to_owned(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}
