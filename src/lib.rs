//! Person-to-person messages.
//!
//! `messages-client` is the page. It keeps no message file. `messages-server`
//! stores every message for this user and sends each new one to the other
//! person's `messages-server`.

pub mod directory;
pub mod log_setup;
pub mod mail;
pub mod node;
pub mod page;
pub mod proto;
pub mod store;

pub const CLIENT_ALIAS: &str = "messages-client";
pub const SERVER_ALIAS: &str = "messages-server";
pub const APP_PROTOCOL: &str = "application/messages";
pub const DEFAULT_CLIENT_PUSH: u16 = 8830;
pub const DEFAULT_SERVER_PUSH: u16 = 8831;
pub const DEFAULT_HTTP_PORT: u16 = 8832;
pub const WEB_SLUG: &str = "messages";
pub const WEB_TITLE: &str = "Messages";
