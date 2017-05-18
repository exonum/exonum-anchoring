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
extern crate iron;
extern crate router;
extern crate iron_test;

use router::Router;
use iron::Headers;
use iron::prelude::*;

use exonum::crypto::HexValue;
use exonum::messages::Message;
use blockchain_explorer::api::Api;

use anchoring_btc_service::api::{AnchoringInfo, LectInfo, PublicApi};
use anchoring_btc_service::details::btc::transactions::BitcoinTx;
use anchoring_btc_service::blockchain::dto::MsgAnchoringUpdateLatest;
use anchoring_btc_sandbox::AnchoringSandbox;
use anchoring_btc_sandbox::helpers::*;

struct ApiSandbox {
    pub router: Router,
}

impl ApiSandbox {
    fn new(anchoring_sandbox: &AnchoringSandbox) -> ApiSandbox {
        let mut router = Router::new();
        let api = PublicApi { blockchain: anchoring_sandbox.blockchain_ref().clone() };
        api.wire(&mut router);

        ApiSandbox { router: router }
    }

    fn request_get<A: AsRef<str>>(&self, route: A) -> IronResult<Response> {
        info!("GET request:'{}'",
              format!("http://127.0.0.1:8000/{}", route.as_ref()));
        iron_test::request::get(&format!("http://127.0.0.1:8000/{}", route.as_ref()),
                                Headers::new(),
                                &self.router)
    }

    fn get_current_lect(&self) -> Option<AnchoringInfo> {
        let response = self.request_get("/api/v1/anchoring/current_lect/").unwrap();
        let body = response_body(response);
        serde_json::from_value(body).unwrap()
    }

    pub fn get_current_lect_of_validator(&self, validator_id: u32) -> LectInfo {
        let response = self.request_get(format!("/api/v1/anchoring/current_lect/{}", validator_id))
            .unwrap();
        let body = response_body(response);
        serde_json::from_value(body).unwrap()
    }
}

fn response_body(response: Response) -> serde_json::Value {
    if let Some(mut body) = response.body {
        let mut buf = Vec::new();
        body.write_body(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        debug!("Received response body:'{}'", &s);
        serde_json::from_str(&s).unwrap()
    } else {
        serde_json::Value::Null
    }
}

// Test normal api usage
#[test]
fn test_api_public_common() {
    init_logger();

    let sandbox = AnchoringSandbox::initialize(&[]);
    anchor_first_block(&sandbox);

    let lects = (0..4)
        .map(|idx| gen_service_tx_lect(&sandbox, idx, &sandbox.latest_anchored_tx(), 1))
        .collect::<Vec<_>>();

    let api_sandbox = ApiSandbox::new(&sandbox);
    let anchoring_info = AnchoringInfo::from(lects[0].tx());
    assert_eq!(api_sandbox.get_current_lect(), Some(anchoring_info));
    // Check validators lects
    for (id, lect) in lects.iter().enumerate() {
        let lect_info = LectInfo {
            hash: Message::hash(lect),
            content: AnchoringInfo::from(lect.tx()),
        };
        assert_eq!(api_sandbox.get_current_lect_of_validator(id as u32),
                   lect_info);
    }
}

// Try to get lect from nonexistent validator id
// result: Panic
#[test]
#[should_panic(expected = "Unknown validator id")]
fn test_api_public_get_lect_nonexistent_validator() {
    init_logger();

    let sandbox = AnchoringSandbox::initialize(&[]);
    let api_sandbox = ApiSandbox::new(&sandbox);
    api_sandbox.get_current_lect_of_validator(100);
}

// Try to get current lect when lects is different.
// result: Returns null
#[test]
fn test_api_public_get_lect_unavailable() {
    init_logger();

    let sandbox = AnchoringSandbox::initialize(&[]);

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
    let lects = (0..2)
        .map(|id| {
                 MsgAnchoringUpdateLatest::new(&sandbox.p(id as usize),
                                               id,
                                               lect_tx.clone(),
                                               lects_count(&sandbox, id),
                                               sandbox.s(id as usize))
             })
        .collect::<Vec<_>>();
    force_commit_lects(&sandbox, lects);

    let api_sandbox = ApiSandbox::new(&sandbox);
    assert_eq!(api_sandbox.get_current_lect(), None);
}