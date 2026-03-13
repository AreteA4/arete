//! # HyperStack
//!
//! Real-time streaming data pipelines for Solana - transform on-chain events
//! into typed state projections.
//!
//! ## Features
//!
//! - **`interpreter`** (default) - AST transformation runtime and VM
//! - **`macros`** (default) - Proc-macros for defining stream specifications
//! - **`server`** (default) - WebSocket server and projection handlers
//! - **`sdk`** - Rust client for connecting to HyperStack servers
//!
//! ## Quick Start
//!
//! ```toml
//! [dependencies]
//! hyperstack = "0.2"
//! ```
//!
//! Or with specific features:
//!
//! ```toml
//! [dependencies]
//! hyperstack = { version = "0.1", features = ["full"] }
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use hyperstack::prelude::*;
//!
//! #[hyperstack(idl = "idl.json")]
//! pub mod my_stream {
//!     #[entity(name = "MyEntity")]
//!     #[derive(Stream)]
//!     struct Entity {
//!         #[map(from = "MyAccount", field = "value")]
//!         pub value: u64,
//!     }
//! }
//! ```

// Re-export interpreter (AST runtime and VM)
#[cfg(feature = "interpreter")]
pub use hyperstack_interpreter as interpreter;

// Re-export macros
#[cfg(feature = "macros")]
pub use hyperstack_macros as macros;

// Re-export server components
#[cfg(feature = "server")]
pub use hyperstack_server as server;

// Re-export SDK client
#[cfg(feature = "sdk")]
pub use hyperstack_sdk as sdk;

#[cfg(feature = "runtime")]
#[doc(hidden)]
pub mod runtime {
    pub use anyhow;
    pub use bs58;
    pub use bytemuck;
    pub use dotenvy;
    pub use hyperstack_interpreter;
    pub use hyperstack_server;
    pub use reqwest;
    pub use serde;
    pub use serde_json;
    pub use smallvec;
    pub use tokio;
    pub use futures;
    pub use tracing;
    pub use yellowstone_vixen;
    pub use yellowstone_vixen_core;
    pub use yellowstone_vixen_yellowstone_grpc_source;

    pub mod serde_helpers {
        pub mod pubkey_base58 {
            use serde::{Deserialize, Deserializer, Serializer};

            pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&bs58::encode(bytes).into_string())
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
                let s = String::deserialize(d)?;
                let bytes = bs58::decode(&s)
                    .into_vec()
                    .map_err(serde::de::Error::custom)?;
                let arr: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
                    serde::de::Error::custom(format!("expected 32 bytes, got {}", v.len()))
                })?;
                Ok(arr)
            }
        }

        /// Serde helper for arrays larger than 32 elements.
        ///
        /// serde's derive macro doesn't support const generics for arrays > 32.
        /// This module serializes arrays as sequences (like Vec) and deserializes
        /// them back into fixed-size arrays.
        ///
        /// Usage: `#[serde(with = "hyperstack::runtime::serde_helpers::big_array")]`
        pub mod big_array {
            use serde::{
                de::{Deserialize, Deserializer, Error, SeqAccess, Visitor},
                ser::{Serialize, SerializeSeq, Serializer},
            };
            use std::fmt;
            use std::marker::PhantomData;

            /// Serialize a fixed-size array as a sequence.
            pub fn serialize<S, T, const N: usize>(
                arr: &[T; N],
                serializer: S,
            ) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
                T: Serialize,
            {
                let mut seq = serializer.serialize_seq(Some(N))?;
                for elem in arr.iter() {
                    seq.serialize_element(elem)?;
                }
                seq.end()
            }

            /// Deserialize a sequence into a fixed-size array.
            pub fn deserialize<'de, D, T, const N: usize>(
                deserializer: D,
            ) -> Result<[T; N], D::Error>
            where
                D: Deserializer<'de>,
                T: Deserialize<'de> + Default + Copy,
            {
                struct ArrayVisitor<T, const N: usize>(PhantomData<T>);

                impl<'de, T, const N: usize> Visitor<'de> for ArrayVisitor<T, N>
                where
                    T: Deserialize<'de> + Default + Copy,
                {
                    type Value = [T; N];

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        write!(formatter, "an array of {} elements", N)
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        let mut arr = [T::default(); N];
                        for (i, elem) in arr.iter_mut().enumerate() {
                            *elem = seq
                                .next_element()?
                                .ok_or_else(|| Error::invalid_length(i, &self))?;
                        }
                        Ok(arr)
                    }
                }

                deserializer.deserialize_seq(ArrayVisitor::<T, N>(PhantomData))
            }

            #[cfg(test)]
            mod tests {
                use serde::{Deserialize, Serialize};

                #[derive(Debug, PartialEq, Serialize, Deserialize)]
                struct LargeArrayStruct {
                    #[serde(with = "super")]
                    data: [u64; 70],
                }

                #[derive(Debug, PartialEq, Serialize, Deserialize)]
                struct NestedLargeArray {
                    name: String,
                    #[serde(with = "super")]
                    values: [u8; 128],
                }

                #[test]
                fn test_serialize_large_array() {
                    let s = LargeArrayStruct { data: [42u64; 70] };
                    let json = serde_json::to_string(&s).unwrap();

                    // Should serialize as JSON array
                    assert!(json.starts_with("{\"data\":["));
                    assert!(json.contains("42"));
                }

                #[test]
                fn test_deserialize_large_array() {
                    let json = r#"{"data":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70]}"#;
                    let s: LargeArrayStruct = serde_json::from_str(json).unwrap();

                    assert_eq!(s.data[0], 1);
                    assert_eq!(s.data[69], 70);
                    assert_eq!(s.data.len(), 70);
                }

                #[test]
                fn test_roundtrip_large_array() {
                    let original = LargeArrayStruct {
                        data: {
                            let mut arr = [0u64; 70];
                            for (i, v) in arr.iter_mut().enumerate() {
                                *v = i as u64;
                            }
                            arr
                        },
                    };

                    let json = serde_json::to_string(&original).unwrap();
                    let restored: LargeArrayStruct = serde_json::from_str(&json).unwrap();

                    assert_eq!(original, restored);
                }

                #[test]
                fn test_nested_struct_with_large_array() {
                    let original = NestedLargeArray {
                        name: "test".to_string(),
                        values: [255u8; 128],
                    };

                    let json = serde_json::to_string(&original).unwrap();
                    let restored: NestedLargeArray = serde_json::from_str(&json).unwrap();

                    assert_eq!(original, restored);
                }

                #[test]
                fn test_deserialize_wrong_length_fails() {
                    // Only 69 elements instead of 70
                    let json = r#"{"data":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69]}"#;
                    let result: Result<LargeArrayStruct, _> = serde_json::from_str(json);

                    assert!(result.is_err());
                }
            }
        }
    }
}

pub mod resolvers {
    pub use hyperstack_interpreter::resolvers::TokenMetadata;
}

/// Prelude module for convenient imports
pub mod prelude {
    // Re-export commonly used items from interpreter
    #[cfg(feature = "interpreter")]
    pub use hyperstack_interpreter::{
        ast::{SerializableStreamSpec, TypedStreamSpec},
        compiler::MultiEntityBytecode,
        vm::VmContext,
        Mutation, UpdateContext,
    };

    #[cfg(feature = "interpreter")]
    pub use hyperstack_interpreter::resolvers::TokenMetadata;

    #[cfg(feature = "macros")]
    pub use hyperstack_macros::{hyperstack, Stream};

    // Re-export server components
    #[cfg(feature = "server")]
    pub use hyperstack_server::{bus::BusManager, config::ServerConfig, projector::Projector};

    // Re-export SDK client
    #[cfg(feature = "sdk")]
    pub use hyperstack_sdk::HyperStack;
}
