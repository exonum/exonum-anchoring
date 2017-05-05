#[macro_use]
extern crate exonum;
extern crate sandbox;
extern crate anchoring_btc_service;
#[macro_use]
extern crate anchoring_btc_sandbox;
extern crate serde;
extern crate serde_json;
extern crate bitcoin;
extern crate bitcoinrpc;
extern crate secp256k1;
extern crate blockchain_explorer;
#[macro_use]
extern crate log;

use serde_json::value::ToJson;
use bitcoin::util::base58::ToBase58;

use exonum::crypto::HexValue;
use exonum::messages::{Message, RawTransaction};
use exonum::storage::{StorageValue, Fork};
use sandbox::sandbox_tests_helper::{SandboxState, add_one_height_with_transactions,
                                    add_one_height_with_transactions_from_other_validator};
use sandbox::sandbox::Sandbox;
use sandbox::config_updater::TxConfig;

use anchoring_btc_service::details::sandbox::{SandboxClient, Request};
use anchoring_btc_service::blockchain::dto::MsgAnchoringUpdateLatest;
use anchoring_btc_service::blockchain::schema::AnchoringSchema;
use anchoring_btc_service::{AnchoringConfig, ANCHORING_SERVICE_ID};
use anchoring_btc_service::error::HandlerError;
use anchoring_btc_service::details::btc::transactions::BitcoinTx;
use anchoring_btc_sandbox::{AnchoringSandboxState, initialize_anchoring_sandbox};
use anchoring_btc_sandbox::helpers::*;

/// Generates a configuration that excludes `sandbox node` from consensus.
/// Then it continues to work as auditor.
fn gen_following_cfg(sandbox: &Sandbox,
                     anchoring_state: &mut AnchoringSandboxState,
                     from_height: u64)
                     -> (RawTransaction, AnchoringConfig) {
    let (_, anchoring_addr) = anchoring_state.common.redeem_script();

    let mut service_cfg = anchoring_state.common.clone();
    let priv_keys = anchoring_state.priv_keys(&anchoring_addr);
    service_cfg.validators.swap_remove(0);

    let following_addr = service_cfg.redeem_script().1;
    for (id, ref mut node) in anchoring_state.nodes.iter_mut().enumerate() {
        node.private_keys
            .insert(following_addr.to_base58check(), priv_keys[id].clone());
    }

    let mut cfg = sandbox.cfg();
    cfg.actual_from = from_height;
    cfg.validators.swap_remove(0);
    *cfg.services
         .get_mut(&ANCHORING_SERVICE_ID.to_string())
         .unwrap() = service_cfg.to_json();
    let tx = TxConfig::new(&sandbox.p(0), &cfg.serialize(), from_height, sandbox.s(0));
    (tx.raw().clone(), service_cfg)
}

pub fn force_commit_lects<I>(sandbox: &Sandbox, lects: I)
    where I: IntoIterator<Item = MsgAnchoringUpdateLatest>
{
    let blockchain = sandbox.blockchain_ref();
    let changes = {
        let view = blockchain.view();
        let anchoring_schema = AnchoringSchema::new(&view);
        for lect_msg in lects {
            anchoring_schema
                .add_lect(lect_msg.validator(), lect_msg.tx().clone())
                .unwrap();
        }
        view.changes()
    };
    blockchain.merge(&changes).unwrap();
}

// Invoke this method after anchor_first_block_lect_normal
pub fn exclude_node_from_validators(sandbox: &Sandbox,
                                    client: &SandboxClient,
                                    sandbox_state: &mut SandboxState,
                                    anchoring_state: &mut AnchoringSandboxState) {
    let cfg_change_height = 12;
    let (cfg_tx, following_cfg) = gen_following_cfg(&sandbox, anchoring_state, cfg_change_height);
    let (_, following_addr) = following_cfg.redeem_script();

    // Check insufficient confirmations case
    let anchored_tx = anchoring_state.latest_anchored_tx().clone();
    client.expect(vec![gen_confirmations_request(anchored_tx.clone(), 10)]);
    add_one_height_with_transactions(&sandbox, &sandbox_state, &[cfg_tx]);

    // Check enough confirmations case
    client.expect(vec![gen_confirmations_request(anchored_tx.clone(), 100),
                       request! {
                            method: "listunspent",
                            params: [0, 9999999, [following_addr]],
                            response: []
                       }]);

    let following_multisig = following_cfg.redeem_script();
    let (_, signatures) =
        anchoring_state.gen_anchoring_tx_with_signatures(&sandbox,
                                                         0,
                                                         anchored_tx.payload().1,
                                                         &[],
                                                         None,
                                                         &following_multisig.1);
    let transition_tx = anchoring_state.latest_anchored_tx().clone();

    add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
    sandbox.broadcast(signatures[0].clone());

    client.expect(vec![gen_confirmations_request(transition_tx.clone(), 100)]);
    add_one_height_with_transactions(&sandbox, &sandbox_state, &signatures);

    let lects = (0..4)
        .map(|id| {
                 gen_service_tx_lect(&sandbox, id, &transition_tx, 2)
                     .raw()
                     .clone()
             })
        .collect::<Vec<_>>();
    sandbox.broadcast(lects[0].clone());
    add_one_height_with_transactions(&sandbox, &sandbox_state, &lects);

    for _ in sandbox.current_height()..cfg_change_height {
        add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
    }

    anchoring_state.common = following_cfg;
    client.expect(vec![request! {
                            method: "getrawtransaction",
                            params: [&transition_tx.txid(), 0],
                            response: transition_tx.to_hex()
                        }]);
    add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);

    assert_eq!(anchoring_state.handler().errors, Vec::new());
}

// We exclude sandbox node from validators
// problems: None
// result: success
#[test]
fn test_auditing_exclude_node_from_validators() {
    let _ = ::blockchain_explorer::helpers::init_logger();

    let (sandbox, client, mut anchoring_state) = initialize_anchoring_sandbox(&[]);
    let mut sandbox_state = SandboxState::new();

    anchor_first_block(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    anchor_first_block_lect_normal(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    exclude_node_from_validators(&sandbox, &client, &mut sandbox_state, &mut anchoring_state);
}

// We lost consensus in lects
// result: Error LectNotFound occured
#[test]
fn test_auditing_lost_consensus_in_lects() {
    let _ = ::blockchain_explorer::helpers::init_logger();

    let (sandbox, client, mut anchoring_state) = initialize_anchoring_sandbox(&[]);
    let mut sandbox_state = SandboxState::new();

    anchor_first_block(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    anchor_first_block_lect_normal(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    exclude_node_from_validators(&sandbox, &client, &mut sandbox_state, &mut anchoring_state);

    for _ in sandbox.current_height()..anchoring_state.nearest_check_lect_height(&sandbox) {
        add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);
    }

    let lect_tx = BitcoinTx::from(anchoring_state.common.funding_tx.clone().0);
    let lect = MsgAnchoringUpdateLatest::new(&sandbox.p(0),
                                             0,
                                             lect_tx,
                                             lects_count(&sandbox, 0),
                                             sandbox.s(0));
    add_one_height_with_transactions_from_other_validator(&sandbox,
                                                          &sandbox_state,
                                                          &[lect.raw().clone()]);

    assert_eq!(anchoring_state.take_errors()[0],
               HandlerError::LectNotFound { height: 10 });
}

// FundingTx from lect not found in `bitcoin` network
// result: Error IncorrectLect occured
#[test]
fn test_auditing_lects_lost_funding_tx() {
    let _ = ::blockchain_explorer::helpers::init_logger();

    let (sandbox, client, mut anchoring_state) = initialize_anchoring_sandbox(&[]);
    let mut sandbox_state = SandboxState::new();

    anchor_first_block(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    anchor_first_block_lect_normal(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    exclude_node_from_validators(&sandbox, &client, &mut sandbox_state, &mut anchoring_state);

    for _ in sandbox.current_height()..anchoring_state.nearest_check_lect_height(&sandbox) {
        add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);
    }

    let lect_tx = BitcoinTx::from(anchoring_state.common.funding_tx.clone().0);
    let lects = (0..3)
        .map(|id| {
                 MsgAnchoringUpdateLatest::new(&sandbox.p(id as usize),
                                               id,
                                               lect_tx.clone(),
                                               lects_count(&sandbox, id),
                                               sandbox.s(id as usize))
             })
        .collect::<Vec<_>>();
    force_commit_lects(&sandbox, lects);

    client.expect(vec![request! {
            method: "getrawtransaction",
            params: [&lect_tx.txid(), 0],
            error: RpcError::NoInformation("Unable to find tx".to_string())
        }]);
    add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);

    assert_eq!(anchoring_state.take_errors()[0],
               HandlerError::IncorrectLect {
                   reason: String::from("Initial funding_tx not found in the bitcoin blockchain"),
                   tx: lect_tx.into(),
               });
}

// FundingTx from lect has no correct outputs
// result: Error IncorrectLect occured
#[test]
fn test_auditing_lects_incorrect_funding_tx() {
    let _ = ::blockchain_explorer::helpers::init_logger();

    let (sandbox, client, mut anchoring_state) = initialize_anchoring_sandbox(&[]);
    let mut sandbox_state = SandboxState::new();

    anchor_first_block(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    anchor_first_block_lect_normal(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    exclude_node_from_validators(&sandbox, &client, &mut sandbox_state, &mut anchoring_state);

    for _ in sandbox.current_height()..anchoring_state.nearest_check_lect_height(&sandbox) {
        add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);
    }

    let lect_tx = BitcoinTx::from_hex("020000000152f2e44424d6cc16ce29566b54468084d1d15329b28e\
                                       8fc7cb9d9d783b8a76d3010000006b4830450221009e5ae44ba558\
                                       6e4aadb9e1bc5369cc9fe9f16c12ff94454ac90414f1c5a3df9002\
                                       20794b24afab7501ba12ea504853a31359d718c2a7ff6dd2688e95\
                                       c5bc6634ce39012102f81d4470a303a508bf03de893223c89360a5\
                                       d093e3095560b71de245aaf45d57feffffff028096980000000000\
                                       17a914dcfbafb4c432a24dd4b268570d26d7841a20fbbd87e7cc39\
                                       0a000000001976a914b3203ee5a42f8f524d14397ef10b84277f78\
                                       4b4a88acd81d1100")
            .unwrap();
    let lects = (0..3)
        .map(|id| {
                 MsgAnchoringUpdateLatest::new(&sandbox.p(id as usize),
                                               id,
                                               lect_tx.clone(),
                                               lects_count(&sandbox, id),
                                               sandbox.s(id as usize))
             })
        .collect::<Vec<_>>();
    force_commit_lects(&sandbox, lects);

    add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);

    assert_eq!(anchoring_state.take_errors()[0],
               HandlerError::IncorrectLect {
                   reason: String::from("Initial funding_tx from cfg is different than in lect"),
                   tx: lect_tx.into(),
               });
}

// Current lect not found in `bitcoin` network
// result: Error IncorrectLect occured
#[test]
fn test_auditing_lects_lost_current_lect() {
    let _ = ::blockchain_explorer::helpers::init_logger();

    let (sandbox, client, mut anchoring_state) = initialize_anchoring_sandbox(&[]);
    let mut sandbox_state = SandboxState::new();

    anchor_first_block(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    anchor_first_block_lect_normal(&sandbox, &client, &sandbox_state, &mut anchoring_state);
    exclude_node_from_validators(&sandbox, &client, &mut sandbox_state, &mut anchoring_state);

    for _ in sandbox.current_height()..anchoring_state.nearest_check_lect_height(&sandbox) {
        add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);
    }

    let lect_tx = anchoring_state.latest_anchored_tx().clone();
    client.expect(vec![request! {
            method: "getrawtransaction",
            params: [&lect_tx.txid(), 0],
            error: RpcError::NoInformation("Unable to find tx".to_string())
        }]);
    add_one_height_with_transactions_from_other_validator(&sandbox, &sandbox_state, &[]);

    assert_eq!(anchoring_state.take_errors()[0],
               HandlerError::IncorrectLect {
                   reason: String::from("Lect not found in the bitcoin blockchain"),
                   tx: lect_tx.into(),
               });
}