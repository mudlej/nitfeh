//! nitfeh compresses lecture audio: any video or audio file becomes a small mono Opus file.

pub mod cli;
pub mod engine;
#[cfg(feature = "gui")]
pub mod gui;
pub mod preset;
pub mod progress;
pub mod runner;
pub mod scan;
pub mod text;
