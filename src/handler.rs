// Copyright 2018 The Exonum Team
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

use exonum::blockchain::ServiceContext;
use exonum::helpers::ValidatorId;

use btc_transaction_utils::p2wsh;
use btc_transaction_utils::TxInRef;
use failure;

use std::cmp;
use std::collections::HashMap;

use blockchain::data_layout::TxInputId;
use blockchain::transactions::TxSignature;
use blockchain::{BtcAnchoringSchema, BtcAnchoringState};
use btc::{Address, Privkey};
use rpc::BtcRelay;

/// The goal of this task is to create anchoring transactions for the corresponding heights.
pub struct UpdateAnchoringChainTask<'a> {
    context: &'a ServiceContext,
    anchoring_state: BtcAnchoringState,
    private_keys: &'a HashMap<Address, Privkey>,
}

impl<'a> UpdateAnchoringChainTask<'a> {
    /// Creates the anchoring chain updater for the given context and private keys.
    pub fn new(
        context: &'a ServiceContext,
        private_keys: &'a HashMap<Address, Privkey>,
    ) -> UpdateAnchoringChainTask<'a> {
        UpdateAnchoringChainTask {
            context,
            anchoring_state: BtcAnchoringSchema::new(context.snapshot()).actual_state(),
            private_keys,
        }
    }

    /// For validators this method creates an Exonum transaction with the signature for
    /// the corresponding anchoring transaction if there is such a need.
    pub fn run(self) -> Result<(), failure::Error> {
        if let Some(validator_id) = self.context.validator_id() {
            let address = self.anchoring_state.output_address();

            let privkey = self
                .private_keys
                .get(&address)
                .ok_or_else(|| format_err!("Private key for the address {} is absent.", address))?;

            self.handle_as_validator(validator_id, &privkey)
        } else {
            self.handle_as_auditor()
        }
    }

    fn handle_as_validator(
        self,
        validator_id: ValidatorId,
        privkey: &Privkey,
    ) -> Result<(), failure::Error> {
        let schema = BtcAnchoringSchema::new(self.context.snapshot());
        let latest_anchored_height = schema.latest_anchored_height();
        let anchoring_height = self
            .anchoring_state
            .following_anchoring_height(latest_anchored_height);

        if self.context.height() < anchoring_height {
            return Ok(());
        }

        // Creates anchoring proposal.
        let (proposal, proposal_inputs) =
            if let Some(proposal) = schema.proposed_anchoring_transaction(&self.anchoring_state) {
                proposal?
            } else {
                return Ok(());
            };

        let config = self.anchoring_state.actual_configuration();
        let redeem_script = config.redeem_script();
        // Creates `Signature` transactions.
        let pubkey = redeem_script.content().public_keys[validator_id.0 as usize];
        let mut signer = p2wsh::InputSigner::new(redeem_script);

        for (index, proposal_input) in proposal_inputs.iter().enumerate() {
            let input_id = TxInputId::new(proposal.id(), index as u32);

            if let Some(input_signatures) = schema.transaction_signatures().get(&input_id) {
                if input_signatures.contains(validator_id) {
                    trace!(
                        " {:?} is already signed by validator {}",
                        input_id,
                        validator_id
                    );
                    continue;
                }
            }

            let signature = signer.sign_input(
                TxInRef::new(proposal.as_ref(), index),
                proposal_inputs[index].as_ref(),
                privkey.0.secret_key(),
            )?;

            signer
                .verify_input(
                    TxInRef::new(proposal.as_ref(), index),
                    proposal_input.as_ref(),
                    &pubkey,
                    &signature,
                ).unwrap();

            self.context.broadcast_transaction(TxSignature {
                validator: validator_id,
                transaction: proposal.clone(),
                input: index as u32,
                input_signature: signature.into(),
            });
        }

        Ok(())
    }

    fn handle_as_auditor(self) -> Result<(), failure::Error> {
        // TODO Think about corresponding business logic.
        Ok(())
    }
}

/// The goal of this task is to push uncommitted anchoring transactions to the Bitcoin blockchain.
#[derive(Debug)]
pub struct SyncWithBtcRelayTask<'a> {
    context: &'a ServiceContext,
    relay: &'a dyn BtcRelay,
}

impl<'a> SyncWithBtcRelayTask<'a> {
    /// Creates synchronization task instance for the given context and the Bitcoin RPC relay.
    pub fn new(context: &'a ServiceContext, relay: &'a dyn BtcRelay) -> SyncWithBtcRelayTask<'a> {
        SyncWithBtcRelayTask { context, relay }
    }

    /// Performs anchoring transactions synchronization with the Bitcoin blockchain.
    /// That is, it finds the first uncommitted anchoring transaction in the Bitcoin
    /// blockchain and sequentially sends it and the subsequent ones to the Bitcoin mempool.
    pub fn run(self) -> Result<(), failure::Error> {
        let schema = BtcAnchoringSchema::new(self.context.snapshot());
        let sync_interval = cmp::max(1, schema.actual_configuration().anchoring_interval / 2);

        if self.context.height().0 % sync_interval == 0 {
            if let Some(index) = self.find_index_of_first_uncommitted_transaction()? {
                let anchoring_txs = schema.anchoring_transactions_chain();
                for tx in anchoring_txs.iter_from(index) {
                    trace!(
                        "Send anchoring transaction to btc relay: {}",
                        tx.id().to_hex()
                    );
                    self.relay.send_transaction(&tx)?;
                }
            }
        }

        Ok(())
    }

    fn find_index_of_first_uncommitted_transaction(&self) -> Result<Option<u64>, failure::Error> {
        let schema = BtcAnchoringSchema::new(self.context.snapshot());
        let anchoring_txs = schema.anchoring_transactions_chain();

        let anchoring_txs_len = anchoring_txs.len();
        let tx_indices = (0..anchoring_txs_len).rev();
        for index in tx_indices {
            let tx = anchoring_txs.get(index).unwrap();
            let info = self.relay.transaction_info(&tx.prev_tx_id())?;
            if info.is_some() {
                let info = self.relay.transaction_info(&tx.id())?;
                if info.is_none() {
                    return Ok(Some(index));
                }
            }
        }
        Ok(None)
    }
}
