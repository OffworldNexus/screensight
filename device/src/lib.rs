//! Screensight device library.
//!
//! The GPUI/V3D rendering shell lives in the `screensightd` binary; everything
//! that can be exercised without a GPU (identity/persistence, the pairing state
//! machine, the WebSocket protocol, the control socket, mDNS advertisement and
//! the CPU rasteriser) lives here as plain, testable modules.

pub mod control;
pub mod font;
pub mod identity;
pub mod mdns;
pub mod pairing;
pub mod protocol;
pub mod random;
pub mod raster;
pub mod runtime;
pub mod screens;
pub mod server;
pub mod state;
pub mod store;
pub mod touch;

#[cfg(feature = "gui")]
pub mod panel;
