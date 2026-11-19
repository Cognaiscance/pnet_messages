//! Person-to-person messages.
//!
//! `messages-server` stores this user's messages, shows them at `/apps/messages/`,
//! and sends each new one to the other person's `messages-server`. A message
//! is only with someone in this user's contacts.

pub mod directory;
pub mod log_setup;
pub mod mail;
pub mod node;
pub mod page;
pub mod proto;
pub mod store;

pub const SERVER_ALIAS: &str = "messages-server";
pub const APP_PROTOCOL: &str = "application/messages";
pub const DEFAULT_SERVER_PUSH: u16 = 8831;
pub const DEFAULT_HTTP_PORT: u16 = 8832;
pub const WEB_SLUG: &str = "messages";
pub const WEB_TITLE: &str = "Messages";
