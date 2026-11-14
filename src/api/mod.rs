//! `dayzmake api`: build and query an index of the DayZ script API.

/// Steam and DayZ install discovery.
pub mod steam;

/// Index model.
pub mod model;
/// Enforce Script declaration parser.
pub mod parse;
/// Lookups on the index and their text and JSON renderers.
pub mod query;
/// Scanner: comments, docs and conditional compilation.
mod scan;
