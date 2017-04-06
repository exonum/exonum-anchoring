use bitcoin::util::base58::ToBase58;

use exonum::blockchain::NodeState;

use error::Error as ServiceError;
use details::btc::transactions::AnchoringTx;
use blockchain::consensus_storage::AnchoringConfig;
use blockchain::schema::AnchoringSchema;

use super::{AnchoringHandler, MultisigAddress, LectKind};

#[doc(hidden)]
impl AnchoringHandler {
    pub fn handle_transition_state(&mut self,
                                   from: AnchoringConfig,
                                   to: AnchoringConfig,
                                   state: &mut NodeState)
                                   -> Result<(), ServiceError> {
        let multisig: MultisigAddress = {
            let mut multisig = self.multisig_address(&from);
            multisig.addr = to.redeem_script().1;
            multisig
        };
        trace!("Transition state, addr={}, following_config={:#?}",
               multisig.addr.to_base58check(),
               to);

        // Similar we update lect each n blocks
        if state.height() % self.node.check_lect_frequency == 0 {
            // First of all we try to update our lect and actual configuration
            self.update_our_lect(&multisig, state)?;
        }

        // Now if we have anchoring tx proposal we must try to finalize it
        if let Some(proposal) = self.proposal_tx.clone() {
            self.try_finalize_proposal_tx(proposal, &multisig, state)?;
        } else {
            // Or try to create proposal
            match self.collect_lects(state)? {
                LectKind::Anchoring(lect) => {
                    if lect.output_address(multisig.common.network) == multisig.addr {
                        return Ok(());
                    }
                    // check that we have enougth confirmations
                    let confirmations = lect.confirmations(&self.client)?.unwrap_or_else(|| 0);
                    if confirmations >= multisig.common.utxo_confirmations {
                        let height = multisig.common.nearest_anchoring_height(state.height());
                        self.create_proposal_tx(lect, &multisig, height, state)?;
                    } else {
                        warn!("Insufficient confirmations for create transition transaction, \
                               tx={:#?}, confirmations={}",
                              lect,
                              confirmations);
                    }
                }
                LectKind::Funding(_) => panic!("We must not to change genesis configuration!"),
                LectKind::None => {
                    warn!("Unable to reach consensus in a lect");
                }
            }
        }
        Ok(())
    }

    pub fn handle_recovering_state(&mut self,
                                   cfg: AnchoringConfig,
                                   state: &mut NodeState)
                                   -> Result<(), ServiceError> {
        let multisig: MultisigAddress = self.multisig_address(&cfg);
        trace!("Trying to recover tx chain after transition to addr={}",
               multisig.addr.to_base58check());

        if state.height() % self.node.check_lect_frequency == 0 {
            // First of all we try to update our lect and actual configuration
            let lect = self.update_our_lect(&multisig, state)?;
            if lect.is_none() {
                // Check prev lect
                let prev_lect: AnchoringTx = AnchoringSchema::new(state.view())
                    .prev_lect(state.id())?
                    .unwrap()
                    .into();
                let network = multisig.common.network;
                if prev_lect.output_address(network) == multisig.addr {
                    trace!("Resend transition transaction, txid={}", prev_lect.txid());
                    self.client.send_transaction(prev_lect.into())?;
                } else {
                    // Start a new anchoring chain from scratch
                    let lect_id = AnchoringSchema::new(state.view())
                        .lect(state.id())?
                        .id();
                    self.try_create_anchoring_tx_chain(&multisig, Some(lect_id), state)?;
                }
            }
        }
        // Try to finalize new tx chain propose if it exist
        if let Some(proposal) = self.proposal_tx.clone() {
            self.try_finalize_proposal_tx(proposal, &multisig, state)?;
        }
        Ok(())
    }
}
