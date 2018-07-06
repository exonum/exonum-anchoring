// Copyright 2017 The Exonum Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! # Introduction
//!
//! Private blockchain infrastructure necessitates additional measures for
//! accountability of the blockchain validators.
//! In public proof of work blockchains (e.g., Bitcoin), accountability is purely economic and is
//! based on game theory and equivocation or retroactive modifications being economically costly.
//! Not so in private blockchains, where these two behaviors
//! are a real threat per any realistic threat model that assumes
//! that the blockchain is of use not only to the system validators,
//! but also to third parties.
//!
//! This crate implements a protocol for blockchain anchoring onto the Bitcoin blockchain
//! that utilizes the native Bitcoin capabilities of creating multisig([p2sh][1]) transactions.
//! This transactions contains metadata from Exonum blockchain (block's hash on corresponding
//! height) and forms a chain.
//!
//! You can read the details in [specification][2].
//!
//! [1]: https://bitcoin.org/en/glossary/p2sh-multisig
//! [2]: https://github.com/exonum/exonum-doc/blob/master/src/advanced/bitcoin-anchoring.md
//!
//! # Examples
//!
//! Create application with anchoring service
//!
//! ```rust,no_run
//! extern crate exonum;
//! extern crate exonum_btc_anchoring as anchoring;
//! extern crate exonum_configuration as configuration;
//! use exonum::helpers::fabric::NodeBuilder;
//! use exonum::helpers;
//!
//! fn main() {
//!     exonum::crypto::init();
//!     helpers::init_logger().unwrap();
//!     let node = NodeBuilder::new()
//!        .with_service(Box::new(configuration::ServiceFactory))
//!        .with_service(Box::new(anchoring::ServiceFactory));
//!     node.run();
//! }
//! ```
//!

#![deny(missing_docs, missing_debug_implementations)]

extern crate bitcoin;
extern crate btc_transaction_utils;
extern crate byteorder;
#[macro_use]
extern crate display_derive;
extern crate exonum_bitcoinrpc as bitcoinrpc;
extern crate failure;
#[macro_use]
extern crate failure_derive;
#[macro_use]
extern crate log;
extern crate secp256k1;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
extern crate toml;

#[macro_use]
extern crate exonum;
extern crate iron;
extern crate rand;
extern crate router;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

pub mod api;
pub mod blockchain;
pub mod cmd;
#[doc(hidden)]
pub mod details;
#[doc(hidden)]
pub mod error;
#[doc(hidden)]
pub mod handler;
#[doc(hidden)]
pub mod local_storage;
pub mod observer;
#[doc(hidden)]
pub mod service;

pub use blockchain::consensus_storage::AnchoringConfig;
pub use cmd::AnchoringServiceFactory as ServiceFactory;
pub use details::btc::{gen_btc_keypair, gen_btc_keypair_with_rng, Network as BitcoinNetwork};
pub use details::rpc::{AnchoringRpcConfig, BitcoinRelay, RpcClient};
pub use error::Error;
pub use handler::AnchoringHandler;
pub use local_storage::AnchoringNodeConfig;
pub use service::{
    gen_anchoring_testnet_config, gen_anchoring_testnet_config_with_rng, AnchoringService,
    ANCHORING_SERVICE_ID, ANCHORING_SERVICE_NAME,
};

#[doc(hidden)]
pub fn majority_count(cnt: u8) -> u8 {
    cnt * 2 / 3 + 1
}
