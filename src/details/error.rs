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

use std::io;

use details::rpc::Error as RpcError;

/// Service error.
#[derive(Debug, Display, Fail)]
pub enum Error {
    /// Rpc error.
    #[display(fmt = "{}", _0)]
    Rpc(RpcError),
    /// Insufficient funds to create anchoring transaction.
    #[display(fmt = "Insufficient funds to create anchoring transaction.")]
    InsufficientFunds,
    /// An input output error.
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}
