//! `anarchy`, a command line tool for inspecting and editing a running anarchy world
//! through its `WorldSocket`.  Build it with `--features socket`.
//!
//! Run a command with `anarchy <command> [args]`, or start a shell with no command.

#[cfg(not(target_arch = "wasm32"))]
#[path = "anarchy/cli.rs"]
mod cli;

#[cfg(not(target_arch = "wasm32"))]
fn main() { cli::main() }

#[cfg(target_arch = "wasm32")]
fn main() { eprintln!("the anarchy CLI needs OS sockets, which wasm doesn't have") }
