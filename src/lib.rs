//! Fastpotify's internals, exposed so diagnostics and tests can reach them.

pub mod api;
pub mod app;
pub mod auth;
pub mod backend;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod images;
#[cfg(target_os = "macos")]
pub mod mac_menu;
pub mod model;
#[cfg(target_os = "linux")]
pub mod mpris;
#[cfg(not(target_os = "linux"))]
#[path = "media_native.rs"]
pub mod mpris;
pub mod paths;
pub mod player;
pub mod settings;
pub mod single_instance;
pub mod sink;
pub mod system_fonts;
pub mod theme;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_native.rs"]
pub mod tray;
pub mod ui;
pub mod updates;
pub mod util;
pub mod zeroconf;
