//! ObserveCraft's connectors (docs/adr/0007-connectors.md, docs/connectors.md): packages that describe how to poll a
//! vendor's API, and the engine that runs them. This crate has no database or Kafka code: the server
//! (`oc-api/src/connectors`) stores connections and state, schedules runs and publishes what they return.
//!
//! - [`manifest`]: the package format (the JSON Schema is generated from it, [`schema`]).
//! - [`expr`], [`template`]: the expression language and templates.
//! - [`package`]: a package's files, loaded and validated ([`validate`]).
//! - [`engine`]: runs a stream or the connection test through a [`http::Transport`], with [`auth`].
//! - [`output`]: records as logs, metrics, events and costs, and their Kafka bodies.
//! - [`replay`]: a package's recorded answers as a transport, and the checks of its expectations.

pub mod auth;
pub mod catalog;
pub mod engine;
pub mod expr;
pub mod formats;
pub mod http;
pub mod manifest;
pub mod output;
pub mod package;
pub mod replay;
pub mod request;
pub mod schema;
pub mod template;
pub mod validate;
