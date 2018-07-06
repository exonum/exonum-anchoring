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

use std::collections::HashSet;
use std::sync::mpsc;

use exonum::blockchain::ServiceContext;
use exonum::helpers::{Height, ValidatorId};
use exonum::storage::Snapshot;

use blockchain::consensus_storage::AnchoringConfig;
use blockchain::dto::MsgAnchoringUpdateLatest;
use blockchain::schema::AnchoringSchema;
use details::btc;
use details::btc::transactions::{AnchoringTx, BitcoinTx, FundingTx, TxKind};
use details::rpc::BitcoinRelay;
use error::Error as ServiceError;
use handler::error::Error as HandlerError;
use local_storage::AnchoringNodeConfig;

use super::{AnchoringHandler, AnchoringState, LectKind, MultisigAddress};

impl AnchoringHandler {
    #[doc(hidden)]
    pub fn new(client: Option<Box<BitcoinRelay>>, node: AnchoringNodeConfig) -> AnchoringHandler {
        AnchoringHandler {
            client,
            node,
            proposal_tx: None,
            known_addresses: HashSet::new(),
            errors_sink: None,
        }
    }

    #[doc(hidden)]
    pub fn validator_id(&self, context: &ServiceContext) -> ValidatorId {
        context
            .validator_id()
            .expect("Request `validator_id` only from validator node.")
    }

    #[doc(hidden)]
    pub fn anchoring_key<'a>(
        &self,
        cfg: &'a AnchoringConfig,
        state: &ServiceContext,
    ) -> &'a btc::PublicKey {
        let validator_id = self.validator_id(state);
        &cfg.anchoring_keys[validator_id.0 as usize]
    }

    #[doc(hidden)]
    pub fn client(&self) -> &BitcoinRelay {
        self.client
            .as_ref()
            .expect(
                "Bitcoind client needs to be present \
                 for validator node",
            )
            .as_ref()
    }

    #[doc(hidden)]
    pub fn multisig_address<'a>(&self, common: &'a AnchoringConfig) -> MultisigAddress<'a> {
        let (redeem_script, addr) = common.redeem_script();
        let addr_str = addr.to_string();
        let priv_key = self.node
            .private_keys
            .get(&addr_str)
            .unwrap_or_else(|| panic!("Expected private key for address={}", addr_str))
            .clone();
        MultisigAddress {
            common,
            priv_key,
            redeem_script,
            addr,
        }
    }

    #[doc(hidden)]
    pub fn import_address(&mut self, addr: &btc::Address) -> Result<(), ServiceError> {
        let addr_str = addr.to_string();
        if !self.known_addresses.contains(&addr_str) {
            self.client().watch_address(addr, false)?;

            trace!("Add address to known, addr={}", addr_str);
            self.known_addresses.insert(addr_str);
        }
        Ok(())
    }

    /// Adds a `private_key` for the corresponding anchoring `address`.
    pub fn add_private_key(&mut self, address: &btc::Address, private_key: btc::PrivateKey) {
        self.node
            .private_keys
            .insert(address.to_string(), private_key);
    }

    #[doc(hidden)]
    pub fn set_errors_sink(&mut self, sink: Option<mpsc::Sender<HandlerError>>) {
        self.errors_sink = sink;
    }

    #[doc(hidden)]
    pub fn actual_config(&self, state: &ServiceContext) -> Result<AnchoringConfig, ServiceError> {
        let schema = AnchoringSchema::new(state.snapshot());
        let common = schema.actual_anchoring_config();
        Ok(common)
    }

    #[doc(hidden)]
    pub fn following_config(
        &self,
        state: &ServiceContext,
    ) -> Result<Option<AnchoringConfig>, ServiceError> {
        let schema = AnchoringSchema::new(state.snapshot());
        let cfg = schema.following_anchoring_config();
        Ok(cfg)
    }

    fn following_config_is_transition(
        &self,
        actual_addr: &btc::Address,
        state: &ServiceContext,
    ) -> Result<Option<(AnchoringConfig, btc::Address)>, ServiceError> {
        if let Some(following) = self.following_config(state)? {
            let following_addr = following.redeem_script().1;
            if actual_addr != &following_addr {
                return Ok(Some((following, following_addr)));
            }
        }
        Ok(None)
    }

    #[doc(hidden)]
    pub fn current_state(
        &mut self,
        state: &ServiceContext,
    ) -> Result<AnchoringState, ServiceError> {
        let actual = self.actual_config(state)?;
        let actual_addr = actual.redeem_script().1;
        let anchoring_schema = AnchoringSchema::new(state.snapshot());

        // Ensure that bitcoind watching for the current addr
        self.import_address(&actual_addr)?;

        if state.validator_id().is_none() {
            return Ok(AnchoringState::Auditing { cfg: actual });
        }

        let key = *self.anchoring_key(&actual, state);

        // If we do not have any 'lect', then we have been added
        // later and can only be in the anchoring or recovering state.
        let actual_lect = if let Some(lect) = anchoring_schema.lect(&key) {
            lect
        } else {
            let prev_cfg = anchoring_schema.previous_anchoring_config().unwrap();
            let is_recovering = if let Some(prev_lect) = anchoring_schema.collect_lects(&prev_cfg) {
                match TxKind::from(prev_lect) {
                    TxKind::FundingTx(_) => prev_cfg.redeem_script().1 != actual_addr,
                    TxKind::Anchoring(tx) => tx.script_pubkey() != &actual_addr.script_pubkey(),
                    TxKind::Other(tx) => panic!("Incorrect lect found={:#?}", tx),
                }
            } else {
                true
            };

            if is_recovering {
                let state = AnchoringState::Recovering {
                    prev_cfg,
                    actual_cfg: actual,
                };
                return Ok(state);
            } else {
                return Ok(AnchoringState::Anchoring { cfg: actual });
            }
        };

        // Check that the following cfg exists and its anchoring address is different.
        let result = self.following_config_is_transition(&actual_addr, state)?;
        let state = if let Some((following, following_addr)) = result {
            // Ensure that bitcoind watching for following addr.
            self.import_address(&following_addr)?;

            match TxKind::from(actual_lect) {
                TxKind::Anchoring(lect) => {
                    let lect_script_pubkey = lect.script_pubkey().clone();
                    if lect_script_pubkey == following_addr.script_pubkey() {
                        let confirmations = self.client().get_transaction_confirmations(lect.id())?;
                        // Lect now is transition transaction
                        AnchoringState::Waiting {
                            lect: lect.into(),
                            confirmations,
                        }
                    } else {
                        AnchoringState::Transition {
                            from: actual,
                            to: following,
                        }
                    }
                }
                TxKind::FundingTx(lect) => {
                    debug_assert_eq!(&lect, actual.funding_tx());
                    AnchoringState::Transition {
                        from: actual,
                        to: following,
                    }
                }
                TxKind::Other(tx) => panic!("Incorrect lect found={:#?}", tx),
            }
        } else {
            match TxKind::from(actual_lect) {
                TxKind::FundingTx(tx) => {
                    if tx.find_out(&actual_addr).is_some() {
                        trace!("Checking funding_tx={:?}, txid={}", tx, tx.id());
                        // Wait until funding_tx got enough confirmation
                        let confirmations = self.client().get_transaction_confirmations(tx.id())?;
                        if !is_enough_confirmations(&actual, confirmations) {
                            let state = AnchoringState::Waiting {
                                lect: tx.into(),
                                confirmations,
                            };
                            return Ok(state);
                        }
                        AnchoringState::Anchoring { cfg: actual }
                    } else {
                        AnchoringState::Recovering {
                            prev_cfg: anchoring_schema
                                .previous_anchoring_config()
                                .expect("Previous configuration is absent in recovering state"),

                            actual_cfg: actual,
                        }
                    }
                }
                TxKind::Anchoring(actual_lect) => {
                    let actual_lect_script_pubkey = actual_lect.script_pubkey().clone();
                    // Ensure that we did not miss transition lect
                    if actual_lect_script_pubkey != actual_addr.script_pubkey() {
                        let state = AnchoringState::Recovering {
                            prev_cfg: anchoring_schema
                                .previous_anchoring_config()
                                .expect("Previous configuration is absent in recovering state"),
                            actual_cfg: actual,
                        };
                        return Ok(state);
                    }
                    // If the lect encodes a transition to a new anchoring address,
                    // we need to wait until it reaches enough confirmations.
                    if actual_lect_is_transition(&actual_lect, &anchoring_schema) {
                        let confirmations = self.client()
                            .get_transaction_confirmations(actual_lect.id())?;
                        if !is_enough_confirmations(&actual, confirmations) {
                            let state = AnchoringState::Waiting {
                                lect: actual_lect.into(),
                                confirmations,
                            };
                            return Ok(state);
                        }
                    }

                    AnchoringState::Anchoring { cfg: actual }
                }
                TxKind::Other(tx) => panic!("Incorrect lect found={:?}", tx),
            }
        };
        Ok(state)
    }

    #[doc(hidden)]
    pub fn after_commit(&mut self, state: &ServiceContext) -> Result<(), ServiceError> {
        match self.current_state(state)? {
            AnchoringState::Anchoring { cfg } => self.handle_anchoring_state(&cfg, state),
            AnchoringState::Transition { from, to } => {
                self.handle_transition_state(&from, &to, state)
            }
            AnchoringState::Recovering {
                prev_cfg,
                actual_cfg,
            } => self.handle_recovering_state(&prev_cfg, &actual_cfg, state),
            AnchoringState::Waiting {
                lect,
                confirmations,
            } => self.handle_waiting_state(lect, confirmations),
            AnchoringState::Auditing { cfg } => self.handle_auditing_state(&cfg, state),
            AnchoringState::Broken => panic!("Broken anchoring state detected!"),
        }
    }

    #[doc(hidden)]
    pub fn collect_lects_for_validator(
        &self,
        anchoring_key: &btc::PublicKey,
        anchoring_cfg: &AnchoringConfig,
        state: &ServiceContext,
    ) -> LectKind {
        let anchoring_schema = AnchoringSchema::new(state.snapshot());

        let our_lect = if let Some(lect) = anchoring_schema.lect(anchoring_key) {
            lect
        } else {
            return LectKind::None;
        };

        let mut count = 0;

        let validators_count = state.validators().len() as u32;
        for key in &anchoring_cfg.anchoring_keys {
            let validators_lect = anchoring_schema.lect(key);
            if Some(&our_lect) == validators_lect.as_ref() {
                count += 1;
            }
        }

        if count >= ::majority_count(validators_count as u8) {
            match TxKind::from(our_lect) {
                TxKind::Anchoring(tx) => LectKind::Anchoring(tx),
                TxKind::FundingTx(tx) => LectKind::Funding(tx),
                TxKind::Other(tx) => panic!("Found incorrect lect transaction, content={:#?}", tx),
            }
        } else {
            LectKind::None
        }
    }

    #[doc(hidden)]
    pub fn collect_lects(&self, state: &ServiceContext) -> Result<LectKind, ServiceError> {
        let anchoring_schema = AnchoringSchema::new(state.snapshot());
        let actual_cfg = anchoring_schema.actual_anchoring_config();
        let kind = if let Some(lect) = anchoring_schema.collect_lects(&actual_cfg) {
            match TxKind::from(lect) {
                TxKind::Anchoring(tx) => LectKind::Anchoring(tx),
                TxKind::FundingTx(tx) => LectKind::Funding(tx),
                TxKind::Other(tx) => {
                    let e = HandlerError::IncorrectLect {
                        reason: "Incorrect lect transaction".to_string(),
                        tx,
                    };
                    return Err(e.into());
                }
            }
        } else {
            LectKind::None
        };
        Ok(kind)
    }

    #[doc(hidden)]
    /// We list unspent transaction by 'listunspent' and search among
    /// them only one that prev_hash is exists in our `lects` or it equals first `funding_tx`
    /// if all `lects` have disappeared.
    pub fn find_lect(
        &self,
        multisig: &MultisigAddress,
        state: &ServiceContext,
    ) -> Result<Option<BitcoinTx>, ServiceError> {
        let lects = self.client()
            .unspent_transactions(&multisig.addr)?
            .into_iter()
            .map(|tx| tx.body);
        for lect in lects {
            if self.transaction_is_lect(&lect, multisig, state)? {
                return Ok(Some(lect));
            }
        }
        Ok(None)
    }

    #[doc(hidden)]
    pub fn update_our_lect(
        &mut self,
        multisig: &MultisigAddress,
        state: &ServiceContext,
    ) -> Result<Option<BitcoinTx>, ServiceError> {
        let key = self.anchoring_key(multisig.common, state);
        trace!("Update our lect");
        if let Some(lect) = self.find_lect(multisig, state)? {
            // New lect with different signatures set.
            let (our_lect, lects_count) = {
                let schema = AnchoringSchema::new(state.snapshot());
                let our_lect = schema.lect(key);
                let count = schema.lects(key).len();
                (our_lect, count)
            };

            if Some(&lect) != our_lect.as_ref() {
                self.send_updated_lect(&lect, lects_count, state);
            }

            Ok(Some(lect))
        } else {
            Ok(None)
        }
    }

    #[doc(hidden)]
    pub fn available_funding_tx(
        &self,
        multisig: &MultisigAddress,
    ) -> Result<Option<FundingTx>, ServiceError> {
        let funding_tx = multisig.common.funding_tx();
        // Do not need to check funding_tx to the different address.
        if funding_tx.find_out(&multisig.addr).is_none() {
            return Ok(None);
        }

        trace!(
            "Checking funding_tx={:?}, addr={} availability",
            funding_tx,
            multisig.addr
        );
        if let Some(info) = self.client()
            .unspent_transactions(&multisig.addr)?
            .iter()
            .find(|tx| tx.body.0 == funding_tx.0)
        {
            trace!(
                "available_funding_tx={:?}, confirmations={:?}",
                funding_tx,
                info.confirmations
            );
            return Ok(Some(funding_tx.clone()));
        }
        Ok(None)
    }

    #[doc(hidden)]
    fn transaction_is_lect(
        &self,
        lect: &BitcoinTx,
        multisig: &MultisigAddress,
        state: &ServiceContext,
    ) -> Result<bool, ServiceError> {
        let schema = AnchoringSchema::new(state.snapshot());
        let key = self.anchoring_key(multisig.common, state);

        // Check that we know tx
        if schema.find_lect_position(key, &lect.id()).is_some() {
            return Ok(true);
        }

        let kind = TxKind::from(lect.clone());
        match kind {
            TxKind::FundingTx(tx) => {
                let genesis_cfg = schema.genesis_anchoring_config();
                Ok(genesis_cfg.funding_tx() == &tx)
            }
            TxKind::Anchoring(tx) => {
                if schema.find_lect_position(key, &tx.prev_hash()).is_some() {
                    return Ok(true);
                }

                let txid = tx.prev_hash();
                let prev_lect = if let Some(tx) = self.client().get_transaction(txid)? {
                    tx
                } else {
                    return Ok(false);
                };

                trace!("Check prev lect={:?}", prev_lect);

                let lect_height = match TxKind::from(prev_lect) {
                    TxKind::FundingTx(_) => Height::zero(),
                    TxKind::Anchoring(tx) => tx.payload().block_height,
                    TxKind::Other(_) => return Ok(false),
                };
                let cfg = schema.anchoring_config_by_height(lect_height);

                let mut prev_lect_count = 0;
                for key in &cfg.anchoring_keys {
                    if schema.find_lect_position(key, &txid).is_some() {
                        prev_lect_count += 1;
                    }
                }

                Ok(prev_lect_count >= cfg.majority_count())
            }
            TxKind::Other(_) => Ok(false),
        }
    }

    #[doc(hidden)]
    fn send_updated_lect(&mut self, lect: &BitcoinTx, lects_count: u64, state: &ServiceContext) {
        if self.proposal_tx.is_some() {
            self.proposal_tx = None;
        }

        info!(
            "LECT ====== txid={}, total_count={}",
            lect.id(),
            lects_count
        );

        let lect_msg = MsgAnchoringUpdateLatest::new(
            state.public_key(),
            self.validator_id(state),
            lect.clone(),
            lects_count,
            state.secret_key(),
        );
        state
            .transaction_sender()
            .send(Box::new(lect_msg))
            .expect("Can't send lect transaction.");
    }
}

/// Transition lects cannot be recovered without breaking of current anchoring chain.
fn actual_lect_is_transition<T>(actual_lect: &AnchoringTx, schema: &AnchoringSchema<T>) -> bool
where
    T: AsRef<Snapshot>,
{
    // If tx contains prev_tx_chain it can not be a transition
    if actual_lect.payload().prev_tx_chain.is_some() {
        return false;
    }

    let prev_lect_id = actual_lect.prev_hash();
    let actual_lect_script_pubkey = actual_lect.script_pubkey();

    if let Some(prev_lect) = schema.known_txs().get(&prev_lect_id) {
        match TxKind::from(prev_lect) {
            TxKind::Anchoring(prev_lect) => {
                let prev_lect_script_pubkey = prev_lect.script_pubkey();
                prev_lect_script_pubkey != actual_lect_script_pubkey
            }
            TxKind::FundingTx(tx) => {
                let genesis_cfg = schema.genesis_anchoring_config();
                if &tx == genesis_cfg.funding_tx() {
                    let prev_lect_script_pubkey = genesis_cfg.redeem_script().1.script_pubkey();
                    &prev_lect_script_pubkey != actual_lect_script_pubkey
                } else {
                    false
                }
            }
            TxKind::Other(tx) => panic!("Incorrect prev_lect found={:?}", tx),
        }
    } else {
        false
    }
}

fn is_enough_confirmations(cfg: &AnchoringConfig, confirmations: Option<u64>) -> bool {
    if let Some(confirmations) = confirmations {
        confirmations >= cfg.utxo_confirmations
    } else {
        false
    }
}
