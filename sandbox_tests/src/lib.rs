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

extern crate exonum;
extern crate sandbox;
extern crate btc_anchoring_service;
extern crate serde;
#[macro_use]
extern crate serde_json;
extern crate bitcoin;
extern crate exonum_bitcoinrpc as bitcoinrpc;
extern crate byteorder;
extern crate secp256k1;
extern crate rand;
extern crate libc;

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::cell::{Ref, RefCell, RefMut};

pub use bitcoinrpc::RpcError as JsonRpcError;
pub use bitcoinrpc::Error as RpcError;

use rand::{SeedableRng, StdRng};
use bitcoin::util::base58::{FromBase58, ToBase58};

use exonum::crypto::Hash;
use exonum::messages::{Message, RawTransaction};

use sandbox::sandbox_with_services;
use sandbox::sandbox::Sandbox;
use sandbox::timestamping::TimestampingService;
use sandbox::sandbox_tests_helper::{SandboxState, VALIDATOR_0, add_one_height_with_transactions,
                                    add_one_height_with_transactions_from_other_validator};
use sandbox::config_updater::ConfigUpdateService;

use btc_anchoring_service::{AnchoringConfig, AnchoringNodeConfig, AnchoringRpc, AnchoringService,
                            gen_anchoring_testnet_config_with_rng};
use btc_anchoring_service::details::sandbox::{Request, SandboxClient};
use btc_anchoring_service::details::btc;
use btc_anchoring_service::details::btc::transactions::{AnchoringTx, FundingTx, TransactionBuilder};
use btc_anchoring_service::blockchain::dto::MsgAnchoringSignature;
use btc_anchoring_service::handler::{AnchoringHandler, collect_signatures};
use btc_anchoring_service::error::HandlerError;

#[macro_use]
mod macros;
#[cfg(test)]
mod tests;
pub mod helpers;
pub mod secp256k1_hack;

pub const ANCHORING_VALIDATOR: u16 = VALIDATOR_0;
pub const ANCHORING_FREQUENCY: u64 = 10;
pub const ANCHORING_UTXO_CONFIRMATIONS: u64 = 24;
pub const ANCHORING_FUNDS: u64 = 4000;
pub const CHECK_LECT_FREQUENCY: u64 = 6;

pub struct AnchoringSandboxState {
    pub sandbox_state: SandboxState,
    pub common: AnchoringConfig,
    pub nodes: Vec<AnchoringNodeConfig>,
    pub latest_anchored_tx: Option<(AnchoringTx, Vec<MsgAnchoringSignature>)>,
}

pub struct AnchoringSandbox {
    pub sandbox: Sandbox,
    pub client: AnchoringRpc,
    pub state: RefCell<AnchoringSandboxState>,
    pub handler: Arc<Mutex<AnchoringHandler>>,
}

/// Generates config for 4 validators and 4000 funds
pub fn gen_sandbox_anchoring_config(
    client: &mut AnchoringRpc,
) -> (AnchoringConfig, Vec<AnchoringNodeConfig>) {
    let requests = vec![
        request! {
            method: "importaddress",
            params: ["2NFGToas8B6sXqsmtGwL1H4kC5fGWSpTcYA", "multisig", false, false]
        },
        request! {
            method: "sendtoaddress",
            params: [
                "2NFGToas8B6sXqsmtGwL1H4kC5fGWSpTcYA",
                "0.00004"
            ],
            response: "a788a2f0a369f3985c5f713d985bb1e7bd3dfb8b35f194b39a5f3ae7d709af9a"
        },
        request! {
            method: "getrawtransaction",
            params: [
                "a788a2f0a369f3985c5f713d985bb1e7bd3dfb8b35f194b39a5f3ae7d709af9a",
                0
            ],
            response: "0100000001e56b729856ecd8a9712cb86a8a702bbd05478b0a323f06d2bcfdce373fc9c71b0\
                10000006a4730440220410e697174595270abbf2e2542ce42186ef6d48fc0dcf9a2c26cb639d6d9e89\
                30220735ff3e6f464d426eec6dd5acfda268624ef628aab38124a1a0b82c1670dddd50121032375139\
                6efcc7e842b522b9d95d84a4f0e4663861124150860d0f728c2cc7d56feffffff02a00f00000000000\
                017a914f18eb74087f751109cc9052befd4177a52c9a30a870313d70b000000001976a914eed3fc59a\
                211ef5cbf1986971cae80bcc983d23a88ac35ae1000"
        },
    ];
    client.expect(requests);
    let mut rng: StdRng = SeedableRng::from_seed([1, 2, 3, 4].as_ref());
    gen_anchoring_testnet_config_with_rng(
        client,
        btc::Network::Testnet,
        4,
        ANCHORING_FUNDS,
        &mut rng,
    )
}

impl AnchoringSandbox {
    pub fn initialize<'a, I>(priv_keys: I) -> AnchoringSandbox
    where
        I: IntoIterator<Item = &'a (&'a str, Vec<&'a str>)>,
    {
        let mut client = AnchoringRpc(SandboxClient::default());
        let (mut common, mut nodes) = gen_sandbox_anchoring_config(&mut client);

        let priv_keys = priv_keys.into_iter().collect::<Vec<_>>();
        // Change default anchoring configs
        common.frequency = ANCHORING_FREQUENCY;
        common.utxo_confirmations = ANCHORING_UTXO_CONFIRMATIONS;
        for &&(addr, ref keys) in &priv_keys {
            for (id, key) in keys.iter().enumerate() {
                nodes[id].private_keys.insert(
                    addr.to_string(),
                    btc::PrivateKey::from_base58check(key).unwrap(),
                );
            }
        }

        for node in &mut nodes {
            node.check_lect_frequency = CHECK_LECT_FREQUENCY;
        }

        client.expect(vec![
            request! {
            method: "importaddress",
            params: ["2NFGToas8B6sXqsmtGwL1H4kC5fGWSpTcYA", "multisig", false, false]
        },
        ]);
        let service = AnchoringService::new_with_client(
            AnchoringRpc(client.clone()),
            common.clone(),
            nodes[ANCHORING_VALIDATOR as usize].clone(),
        );
        let service_handler = service.handler();
        let sandbox = sandbox_with_services(vec![
            Box::new(service),
            Box::new(TimestampingService::new()),
            Box::new(ConfigUpdateService::new()),
        ]);

        let state = AnchoringSandboxState {
            sandbox_state: SandboxState::new(),
            common: common,
            nodes: nodes,
            latest_anchored_tx: None,
        };

        AnchoringSandbox {
            state: state.into(),
            handler: service_handler,
            client: client,
            sandbox: sandbox,
        }
    }

    pub fn client(&self) -> &AnchoringRpc {
        &self.client
    }

    pub fn handler(&self) -> MutexGuard<AnchoringHandler> {
        self.handler.lock().unwrap()
    }

    pub fn take_errors(&self) -> Vec<HandlerError> {
        let mut handler = self.handler();
        let v = handler.errors.drain(..).collect::<Vec<_>>();
        v
    }

    pub fn priv_keys(&self, addr: &btc::Address) -> Vec<btc::PrivateKey> {
        self.nodes()
            .iter()
            .map(|cfg| cfg.private_keys[&addr.to_base58check()].clone())
            .collect::<Vec<_>>()
    }

    pub fn nodes(&self) -> Ref<Vec<AnchoringNodeConfig>> {
        Ref::map(self.state.borrow(), |s| &s.nodes)
    }

    pub fn nodes_mut(&self) -> RefMut<Vec<AnchoringNodeConfig>> {
        RefMut::map(self.state.borrow_mut(), |s| &mut s.nodes)
    }

    pub fn current_priv_keys(&self) -> Vec<btc::PrivateKey> {
        self.priv_keys(&self.current_cfg().redeem_script().1)
    }

    pub fn current_cfg(&self) -> Ref<AnchoringConfig> {
        Ref::map(self.state.borrow(), |s| &s.common)
    }

    pub fn current_addr(&self) -> btc::Address {
        self.current_cfg().redeem_script().1
    }

    pub fn set_anchoring_cfg(&self, cfg: AnchoringConfig) {
        self.state.borrow_mut().common = cfg;
    }

    pub fn current_redeem_script(&self) -> btc::RedeemScript {
        self.current_cfg().redeem_script().0
    }

    pub fn current_funding_tx(&self) -> FundingTx {
        self.current_cfg().funding_tx().clone()
    }

    pub fn next_check_lect_height(&self) -> u64 {
        let height = self.sandbox.current_height();
        let frequency = self.state.borrow().nodes[0].check_lect_frequency as u64;
        height - height % frequency + frequency
    }

    pub fn next_anchoring_height(&self) -> u64 {
        let height = self.sandbox.current_height();
        let frequency = self.current_cfg().frequency as u64;
        height - height % frequency + frequency
    }

    pub fn latest_anchoring_height(&self) -> u64 {
        let height = self.sandbox.current_height();
        self.current_cfg().latest_anchoring_height(height)
    }

    pub fn latest_anchored_tx(&self) -> AnchoringTx {
        self.state
            .borrow()
            .latest_anchored_tx
            .as_ref()
            .unwrap()
            .0
            .clone()
    }

    pub fn set_latest_anchored_tx(&self, tx: Option<(AnchoringTx, Vec<MsgAnchoringSignature>)>) {
        self.state.borrow_mut().latest_anchored_tx = tx;
    }

    pub fn latest_anchored_tx_signatures(&self) -> Vec<MsgAnchoringSignature> {
        self.state
            .borrow()
            .latest_anchored_tx
            .as_ref()
            .unwrap()
            .1
            .clone()
    }

    pub fn gen_anchoring_tx_with_signatures(
        &self,
        height: u64,
        block_hash: Hash,
        funds: &[FundingTx],
        prev_tx_chain: Option<btc::TxId>,
        addr: &btc::Address,
    ) -> (AnchoringTx, Vec<RawTransaction>) {
        let (propose_tx, signed_tx, signs) = {
            let (prev_tx, prev_tx_input) = self.state
                .borrow()
                .latest_anchored_tx
                .clone()
                .map(|x| {
                    let tx = (x.0).0;
                    let input = 0;
                    (tx, input)
                })
                .unwrap_or_else(|| {
                    let cfg = self.current_cfg();
                    let tx = cfg.funding_tx().clone();
                    let input = tx.find_out(&cfg.redeem_script().1).unwrap();
                    (tx.0.clone(), input)
                });

            let mut builder = TransactionBuilder::with_prev_tx(&prev_tx, prev_tx_input)
                .payload(height, block_hash)
                .prev_tx_chain(prev_tx_chain)
                .send_to(addr.clone())
                .fee(1000);
            for fund in funds {
                let out = fund.find_out(addr).unwrap();
                builder = builder.add_funds(fund, out);
            }

            let tx = builder.into_transaction().unwrap();
            let signs = self.gen_anchoring_signatures(&tx);
            let signed_tx = self.finalize_tx(tx.clone(), signs.clone());
            (tx, signed_tx, signs)
        };
        self.state.borrow_mut().latest_anchored_tx = Some((signed_tx, signs.clone()));

        let signs = signs
            .into_iter()
            .map(|tx| tx.raw().clone())
            .collect::<Vec<_>>();
        (propose_tx, signs)
    }

    pub fn finalize_tx<I>(&self, tx: AnchoringTx, signs: I) -> AnchoringTx
    where
        I: IntoIterator<Item = MsgAnchoringSignature>,
    {
        let collected_signs = collect_signatures(&tx, &self.current_cfg(), signs).unwrap();
        tx.finalize(&self.current_redeem_script(), collected_signs)
    }

    pub fn gen_anchoring_signatures(&self, tx: &AnchoringTx) -> Vec<MsgAnchoringSignature> {
        let (redeem_script, addr) = self.current_cfg().redeem_script();

        let priv_keys = self.priv_keys(&addr);
        let mut signs = Vec::new();
        for (validator, priv_key) in priv_keys.iter().enumerate() {
            for input in tx.inputs() {
                let signature = tx.sign_input(&redeem_script, input, priv_key);
                signs.push(MsgAnchoringSignature::new(
                    &self.sandbox.service_public_key(validator),
                    validator as u16,
                    tx.clone(),
                    input,
                    &signature,
                    self.sandbox.service_secret_key(validator),
                ));
            }
        }
        signs
    }

    pub fn add_height<'a, I>(&self, txs: I)
    where
        I: IntoIterator<Item = &'a RawTransaction>,
    {
        add_one_height_with_transactions(&self.sandbox, &self.state.borrow().sandbox_state, txs)
    }

    pub fn add_height_as_auditor(&self, txs: &[RawTransaction]) {
        add_one_height_with_transactions_from_other_validator(
            &self.sandbox,
            &self.state.borrow().sandbox_state,
            txs,
        )
    }

    pub fn fast_forward_to_height(&self, height: u64) {
        for _ in self.sandbox.current_height()..height {
            self.add_height(&[]);
        }
    }

    pub fn fast_forward_to_height_as_auditor(&self, height: u64) {
        for _ in self.sandbox.current_height()..height {
            self.add_height_as_auditor(&[]);
        }
    }
}

impl Deref for AnchoringSandbox {
    type Target = Sandbox;

    fn deref(&self) -> &Sandbox {
        &self.sandbox
    }
}

impl DerefMut for AnchoringSandbox {
    fn deref_mut(&mut self) -> &mut Sandbox {
        &mut self.sandbox
    }
}
