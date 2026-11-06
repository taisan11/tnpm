//! Cases are grouped by the behavior they verify; Cargo keeps one test target.
mod aliases;
mod cache;
mod config;
mod extended_install;
mod install;
mod outdated;
mod output;
mod peers;
mod project;
mod registry;
mod runner;
#[cfg(not(windows))]
mod shell_alias;
mod update;
#[cfg(windows)]
mod windows_alias;
