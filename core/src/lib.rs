// SPDX-License-Identifier: AGPL-3.0-only
//! Application core of harbour-threesea.
//!
//! Everything that touches keys, network bytes or plaintext at rest lives here. The Qt side only
//! talks to this crate through the C ABI in [`ffi`] (protocol: docs/ffi.md).
#![deny(unsafe_code)]

pub mod address_book;
pub mod android_backup;
pub mod app;
pub mod app_android_backup;
pub mod app_calls;
pub mod app_chat_export;
pub mod app_contacts;
pub mod app_group_calls;
pub mod app_groups;
pub mod app_media;
pub mod app_messages;
pub mod app_multidevice;
pub mod app_polls;
pub mod app_safe;
pub mod app_spam;
pub mod app_updates;
pub mod app_web;
pub mod app_web_actions;
pub mod app_web_data;
pub mod app_web_manage;
pub mod app_web_media;
pub mod applock;
pub mod archive;
pub mod call_video;
pub mod calls;
pub mod chat_notifications;
pub mod connection;
pub mod contacts;
/// Made-up chats for the emulator build; never part of a device build (see `demo.rs`).
#[cfg(feature = "demo")]
pub mod demo;
pub(crate) mod detached;
pub(crate) mod dial;
pub mod directory;
pub mod distribution_lists;
pub mod ffi;
pub mod file_types;
pub mod fs_store;
pub mod gifs;
pub mod group_call_frames;
pub mod group_call_handshake;
pub mod group_call_sdp;
pub mod group_call_vp8;
pub mod group_calls;
pub mod groups;
pub mod https;
pub mod join;
pub mod links;
pub mod media;
pub mod mentions;
pub mod messages;
pub mod mp4;
pub mod multidevice;
pub mod pins;
pub mod polls;
pub mod profile_pictures;
pub mod providers;
pub mod qr;
pub mod reactions;
pub mod reflected;
pub mod rendezvous;
pub mod runtime;
pub mod safe;
pub mod sdp;
pub mod store;
pub mod sync;
pub mod transport;
pub mod updates;
pub mod voice;
pub mod web;
pub mod websocket;

/// Version of this crate, as reported to the UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
