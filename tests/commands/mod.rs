//! Cases are grouped by the behavior they verify; Cargo keeps one test target.
mod aliases;
mod audit;
mod auth;
mod cache;
mod config;
mod explain;
mod extended_install;
mod install;
mod isolated;
mod jsr;
mod outdated;
mod output;
mod peers;
mod project;
mod publish;
mod registry;
mod remote_imports;
mod runner;
#[cfg(not(windows))]
mod shell_alias;
mod tarball_url;
#[cfg(not(windows))]
mod toolchain;
mod trust_rebuild;
mod update;
#[cfg(windows)]
mod windows_alias;
mod workspace_run;
