//! NoKey: a lightweight native Windows hotkey manager.
//!
//! Layers: `shortcut` and `engine` (pure hotkey logic), `config`,
//! `discord`, `net`, `security`, `update`, `services` (no UI),
//! `win` (OS integration) and `ui` (windows). `app` ties them together.

// windows-sys models handles (HWND, HDC, ...) as raw pointers. They are
// opaque ids that we only pass back to Windows, never dereference.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod actions;
pub mod app;
pub mod config;
pub mod discord;
pub mod engine;
pub mod i18n;
pub mod icon_art;
pub mod net;
pub mod security;
pub mod services;
pub mod shortcut;
pub mod ui;
pub mod update;
pub mod win;
