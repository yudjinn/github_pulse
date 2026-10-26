//! Platform-independent pieces of github_pulse: config, line model, rendering and
//! API response parsing. No I/O, so it builds for both native and wasm32.

pub mod config;

pub use config::Config;
