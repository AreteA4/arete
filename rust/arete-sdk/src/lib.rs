//! Rust client SDK for connecting to Arete streaming servers.
//!
//! ```rust,ignore
//! use arete_sdk::prelude::*;
//! use arete_stacks::ore::OreStack;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let a4 = Arete::<OreStack>::connect().await?;
//!     
//!     for round in a4.views.latest().listen() {
//!         println!("Round: {:?}", round);
//!     }
//!     
//!     Ok(())
//! }
//! ```

mod auth;
mod client;
mod config;
mod connection;
mod entity;
mod error;
mod frame;
pub mod prelude;
pub mod serde_utils;
mod store;
mod stream;
mod subscription;
pub mod view;

pub use auth::{AuthConfig, AuthToken, TokenTransport};
pub use client::{Arete, AreteBuilder};
pub use config::{AreteConfig, ConnectionConfig};
pub use connection::{ConnectionManager, ConnectionState};
pub use entity::Stack;
pub use error::{AreteError, AuthErrorCode, SocketIssue};
pub use frame::{
    parse_frame, parse_snapshot_entities, try_parse_subscribed_frame, Frame, Mode, Operation,
    SnapshotEntity,
};
pub use store::{deep_merge_with_append, SharedStore, StoreConfig, StoreUpdate};
pub use stream::{
    EntityStream, FilterMapStream, FilteredStream, KeyFilter, MapStream, RichEntityStream,
    RichUpdate, Update, UseStream,
};

pub use subscription::{ClientMessage, Subscription, Unsubscription};
pub use view::{
    RichWatchBuilder, StateView, UseBuilder, ViewBuilder, ViewHandle, Views, WatchBuilder,
};
