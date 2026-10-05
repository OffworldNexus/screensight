//! `screensight` — control CLI for a local Screensight display device.
//!
//! Talks to the running `screensightd` daemon over its Unix control socket.
//! Intended to be run over SSH on the Raspberry Pi, as the panel has no
//! physical buttons: `screensight pair`, `unpair`, `select`, `status`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use screensight::control::{self, socket_path};
use screensight::protocol::{ControlRequest, StatusReport};

#[derive(Parser)]
#[command(
    name = "screensight",
    version,
    about = "Control a local Screensight display device"
)]
struct Cli {
    /// Print the raw JSON control response instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show identity, pairing window and paired Home Assistant instances.
    Status,
    /// Open (or re-arm) the pairing window and print the panel code.
    Pair,
    /// Close the pairing window without pairing.
    CancelPair,
    /// Confirm a pending pairing as if the user tapped "Yes" on the panel.
    /// Intended for headless and automated testing.
    Confirm,
    /// Decline a pending pairing as if the user tapped "Not my home".
    Reject,
    /// Forget one paired instance, or every instance with --all.
    Unpair {
        /// The Home Assistant instance id to forget.
        id: Option<String>,
        /// Forget every paired instance.
        #[arg(long)]
        all: bool,
    },
    /// Choose which paired instance drives the display.
    Select {
        /// The Home Assistant instance id to give control to.
        id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let request = match cli.command {
        Command::Status => ControlRequest::Status,
        Command::Pair => ControlRequest::Pair,
        Command::CancelPair => ControlRequest::CancelPair,
        Command::Confirm => ControlRequest::Confirm,
        Command::Reject => ControlRequest::Reject,
        Command::Unpair { id, all } => ControlRequest::Unpair { id, all },
        Command::Select { id } => ControlRequest::Select { id },
    };

    let response = control::request(&socket_path(), &request)?;

    if cli.json {
        println!("{}", serde_json::to_string(&response)?);
        if !response.ok {
            std::process::exit(1);
        }
        return Ok(());
    }

    if !response.ok {
        eprintln!(
            "error: {}",
            response.error.as_deref().unwrap_or("unknown error")
        );
        std::process::exit(1);
    }

    if let Some(status) = response.status {
        print_status(&status);
    } else {
        println!("ok");
    }
    Ok(())
}

fn print_status(status: &StatusReport) {
    println!("Screensight {} ({})", status.model, status.version);
    println!("  id:         {}", status.id);
    println!("  name:       {}", status.name);
    if status.pairing {
        match &status.pairing_code {
            Some(code) => println!("  pairing:    OPEN — code {code}"),
            None => println!("  pairing:    OPEN"),
        }
    } else {
        println!("  pairing:    closed");
    }

    if status.instances.is_empty() {
        println!("  paired:     none");
        return;
    }
    println!("  paired:");
    for instance in &status.instances {
        let marker = if instance.selected { '*' } else { ' ' };
        println!("   {marker} {}  {}", instance.ha_id, instance.ha_name);
    }
}
