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

extern crate bitcoin;
extern crate exonum_bitcoinrpc as bitcoinrpc;
extern crate secp256k1;
extern crate rand;
extern crate serde;
extern crate libc;
extern crate byteorder;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate pretty_assertions;
extern crate exonum;
extern crate exonum_btc_anchoring;
extern crate exonum_testkit;

#[macro_use]
pub mod testkit_extras;

use exonum::messages::Message;
use exonum::helpers::{Height, ValidatorId};
use exonum::encoding::serialize::FromHex;
use exonum_testkit::{ApiKind, TestKitApi};

use exonum_btc_anchoring::ANCHORING_SERVICE_NAME;
use exonum_btc_anchoring::api::{AnchoringInfo, LectInfo};
use exonum_btc_anchoring::observer::AnchoringChainObserver;
use exonum_btc_anchoring::blockchain::dto::MsgAnchoringUpdateLatest;
use exonum_btc_anchoring::details::btc;
use exonum_btc_anchoring::details::btc::transactions::{AnchoringTx, BitcoinTx};
use testkit_extras::{AnchoringTestKit, TestClient};
use testkit_extras::helpers::*;

trait AnchoringApi {
    fn actual_lect(&self) -> Option<AnchoringInfo>;

    fn current_lect_of_validator(&self, id: usize) -> LectInfo;

    fn actual_address(&self) -> btc::Address;

    fn following_address(&self) -> Option<btc::Address>;

    fn nearest_lect(&self, height: u64) -> Option<AnchoringTx>;
}

impl AnchoringApi for TestKitApi {
    fn actual_lect(&self) -> Option<AnchoringInfo> {
        self.get(ApiKind::Service(ANCHORING_SERVICE_NAME), "/v1/actual_lect/")
    }

    fn current_lect_of_validator(&self, id: usize) -> LectInfo {
        self.get(
            ApiKind::Service(ANCHORING_SERVICE_NAME),
            &format!("/v1/actual_lect/{}", id),
        )
    }

    fn actual_address(&self) -> btc::Address {
        self.get(
            ApiKind::Service(ANCHORING_SERVICE_NAME),
            "/v1/address/actual",
        )
    }

    fn following_address(&self) -> Option<btc::Address> {
        self.get(
            ApiKind::Service(ANCHORING_SERVICE_NAME),
            "/v1/address/following",
        )
    }

    fn nearest_lect(&self, height: u64) -> Option<AnchoringTx> {
        self.get(
            ApiKind::Service(ANCHORING_SERVICE_NAME),
            &format!("/v1/nearest_lect/{}", height),
        )
    }
}

// Test normal api usage
#[test]
fn test_api_public_common() {
    let mut testkit = AnchoringTestKit::default();
    anchor_first_block(&mut testkit);

    let lects = (0..4)
        .map(|idx| {
            gen_service_tx_lect(&testkit, ValidatorId(idx), &testkit.latest_anchored_tx(), 1)
        })
        .collect::<Vec<_>>();

    let api = testkit.api();
    let anchoring_info = AnchoringInfo::from(lects[0].tx());
    assert_eq!(api.actual_lect(), Some(anchoring_info));
    // Check validators lects
    for (id, lect) in lects.iter().enumerate() {
        let lect_info = LectInfo {
            hash: Message::hash(lect),
            content: AnchoringInfo::from(lect.tx()),
        };
        assert_eq!(api.current_lect_of_validator(id), lect_info);
    }
}


// Try to get lect from nonexistent validator id
// result: Panic
#[test]
#[should_panic(expected = "Unknown validator id")]
fn test_api_public_get_lect_nonexistent_validator() {
    let testkit = AnchoringTestKit::default();
    let api = testkit.api();
    api.current_lect_of_validator(100);
}

// Try to get current lect when there is no agreed [or consensus] lect.
// result: Returns null
#[test]
fn test_api_public_get_lect_unavailable() {
    let mut testkit = AnchoringTestKit::default();

    let lect_tx = BitcoinTx::from_hex(
        "020000000152f2e44424d6cc16ce29566b54468084d1d15329b28e\
         8fc7cb9d9d783b8a76d3010000006b4830450221009e5ae44ba558\
         6e4aadb9e1bc5369cc9fe9f16c12ff94454ac90414f1c5a3df9002\
         20794b24afab7501ba12ea504853a31359d718c2a7ff6dd2688e95\
         c5bc6634ce39012102f81d4470a303a508bf03de893223c89360a5\
         d093e3095560b71de245aaf45d57feffffff028096980000000000\
         17a914dcfbafb4c432a24dd4b268570d26d7841a20fbbd87e7cc39\
         0a000000001976a914b3203ee5a42f8f524d14397ef10b84277f78\
         4b4a88acd81d1100",
    ).unwrap();
    let lects = (0..2)
        .map(|id| {
            let validator_id = ValidatorId(id);
            let keypair = testkit.validator(validator_id).service_keypair();
            MsgAnchoringUpdateLatest::new(
                keypair.0,
                validator_id,
                lect_tx.clone(),
                lects_count(&testkit, validator_id),
                keypair.1,
            )
        })
        .collect::<Vec<_>>();
    force_commit_lects(&mut testkit, lects);

    let api = testkit.api();
    assert_eq!(api.actual_lect(), None);
}

// Try to get actual anchoring address
#[test]
fn test_api_public_get_current_address() {
    let testkit = AnchoringTestKit::default();
    let api = testkit.api();
    assert_eq!(api.actual_address(), testkit.current_addr());
}

// try to get following address
#[test]
fn test_api_public_get_following_address_existent() {
    let mut testkit = AnchoringTestKit::default();
    let requests = testkit.requests();
    let api = testkit.api();

    let (cfg_proposal, following_cfg) =
        gen_following_cfg_exclude_validator(&mut testkit, Height(10));
    let following_addr = following_cfg.redeem_script().1;

    anchor_first_block(&mut testkit);
    anchor_first_block_lect_normal(&mut testkit);

    requests.expect(vec![
        request! {
            method: "importaddress",
            params: [&following_addr, "multisig", false, false]
        },
        confirmations_request(&testkit.latest_anchored_tx(), 0),
    ]);
    testkit.commit_configuration_change(cfg_proposal);
    testkit.create_block();

    assert_eq!(api.following_address(), Some(following_addr));
}

// try to get following address when it does not exists
// result: Returns null
#[test]
fn test_api_public_get_following_address_nonexistent() {
    let testkit = AnchoringTestKit::default();
    let api = testkit.api();
    assert_eq!(api.following_address(), None);
}

// Testing the observer for the existing anchoring chain.
#[test]
fn test_api_anchoring_observer_normal() {
    let mut testkit = AnchoringTestKit::default();
    let anchoring_addr = testkit.current_addr();

    anchor_first_block(&mut testkit);
    anchor_first_block_lect_normal(&mut testkit);
    // Anchoring transaction for block with height 0.
    let first_anchored_tx = testkit.latest_anchored_tx();

    anchor_second_block_normal(&mut testkit);
    // Anchoring transaction for block with height 10.
    let second_anchored_tx = testkit.latest_anchored_tx();

    let client = TestClient::default();
    let requests = client.requests();
    let mut observer = AnchoringChainObserver::new_with_client(
        testkit.blockchain_mut().clone(),
        Box::new(client),
        0,
    );
    requests.expect(vec![
        request! {
            method: "listunspent",
            params: [0, 9_999_999, [&anchoring_addr]],
            response: [
                listunspent_entry(&second_anchored_tx, &anchoring_addr, 10)
            ]
        },
        get_transaction_request(&second_anchored_tx),
        confirmations_request(&second_anchored_tx, 100),
        get_transaction_request(&first_anchored_tx),
        confirmations_request(&first_anchored_tx, 200),
        get_transaction_request(&testkit.current_funding_tx()),
    ]);
    observer.check_anchoring_chain().unwrap();

    let api = testkit.api();

    // Check that `first_anchored_tx` anchors the block at height 0.
    assert_eq!(api.nearest_lect(0), Some(first_anchored_tx));
    // Check that closest anchoring transaction for height 1 is
    // `second_anchored_tx` that anchors the block at height 10.
    assert_eq!(api.nearest_lect(1), Some(second_anchored_tx));
    // Check that there are no anchoring transactions for heights that greater than 10
    assert_eq!(api.nearest_lect(11), None);
}
