//! User-facing commands built on the shared package-management modules.
pub(crate) mod alias;
pub(crate) mod auth;
pub(crate) mod cache;
pub(crate) mod explain;
pub(crate) mod init;
pub(crate) mod packages;
pub(crate) mod publish;
mod registry;
pub(crate) mod run;
pub(crate) mod upgrade;
pub(crate) mod version;
pub(crate) mod view;
pub(crate) mod workspace_run;
