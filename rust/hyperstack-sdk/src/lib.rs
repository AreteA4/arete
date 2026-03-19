//! Rust client SDK for connecting to HyperStack streaming servers.
//!
//! ```rust,ignore
//! use hyperstack_sdk::prelude::*;
//! use hyperstack_stacks::ore::OreStack;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let hs = HyperStack::<OreStack>::connect().await?;
//!     
//!     for round in hs.views.latest().listen() {
//!         println!("Round: {:?}", round);
//!     }
//!     
//!     Ok(())
//! }
//! ```

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

pub use client::{HyperStack, HyperStackBuilder};
pub use connection::ConnectionState;
pub use entity::Stack;
pub use error::HyperStackError;
pub use frame::{Frame, Mode, Operation};
pub use store::{SharedStore, StoreUpdate};
pub use stream::{
    EntityStream, FilterMapStream, FilteredStream, KeyFilter, MapStream, RichEntityStream,
    RichUpdate, Update, UseStream,
};
pub use subscription::Subscription;
pub use view::{RichWatchBuilder, StateView, UseBuilder, ViewBuilder, ViewHandle, Views, WatchBuilder};
