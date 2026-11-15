//! dayzmake: DayZ modding tools.
//!
//! The `dayzmake` binary's CLI, MCP server and script API index; the file formats live in the
//! `dayzmake-core` crate.

/// Script API index builder and query tool.
pub mod api;
/// stdin/stdout and file plumbing for the commands.
pub mod cli_io;
/// The CLI's warning sink and error printing.
pub mod cli_warnings;
/// The commands behind the CLI and the MCP tools.
pub mod commands;
/// Stdio MCP server exposing the API index and the file commands as tools.
pub mod mcp;
/// CLI definition and command dispatch.
pub mod run;
