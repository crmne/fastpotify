//! Fastpotify's internals, exposed so diagnostics and tests can reach them.

pub mod api;
pub mod app;
pub mod auth;
pub mod backend;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
mod fonts;
pub mod images;
pub mod model;
#[cfg(target_os = "linux")]
pub mod mpris;
#[cfg(not(target_os = "linux"))]
#[path = "mpris_stub.rs"]
pub mod mpris;
pub mod paths;
pub mod player;
pub mod settings;
pub mod single_instance;
pub mod sink;
pub mod theme;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_stub.rs"]
pub mod tray;
pub mod ui;
pub mod util;
pub mod zeroconf;
