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

extern crate rand;

use std::collections::HashMap;

use serde_json;
use rand::Rng;
use bitcoin::network::constants::Network;
use bitcoin::util::base58::{FromBase58, ToBase58};
use bitcoin::util::address::Privkey as RawPrivateKey;
use bitcoin::blockdata::transaction::SigHashType;
use secp256k1::key::PublicKey as RawPublicKey;
use secp256k1::Secp256k1;

use exonum::helpers::{self, Height};
use exonum::crypto::Hash;
use exonum::storage::StorageValue;
use exonum::encoding::serialize::{FromHex, encode_hex};
use exonum::encoding::Field;

use details::btc::transactions::{sign_tx_input, verify_tx_input, AnchoringTx, BitcoinTx,
                                 FundingTx, TransactionBuilder, TxKind};
use details::btc;
use details::btc::HexValueEx;

pub fn dummy_anchoring_tx(redeem_script: &btc::RedeemScript) -> AnchoringTx {
    let addr = btc::Address::from_script(redeem_script, Network::Testnet);
    let input_tx = AnchoringTx::from_hex(
        "01000000019aaf09d7e73a5f9ab394f1358bfb3dbde7b15b983d715f\
         5c98f369a3f0a288a70000000000ffffffff02b80b00000000000017a914f18eb74087f751109cc9052befd417\
         7a52c9a30a8700000000000000002c6a2a012800000000000000007fab6f66a0f7a747c820cd01fa30d7bdebd2\
         6b91c6e03f742abac0b3108134d900000000",
    ).unwrap();
    TransactionBuilder::with_prev_tx(&input_tx, 0)
        .fee(1000)
        .payload(Height::zero(), Hash::zero())
        .send_to(addr)
        .into_transaction()
        .unwrap()
}

pub fn gen_anchoring_keys(count: usize) -> (Vec<btc::PublicKey>, Vec<btc::PrivateKey>) {
    let mut validators = Vec::new();
    let mut priv_keys = Vec::new();
    for _ in 0..count {
        let (pub_key, priv_key) = btc::gen_btc_keypair(Network::Testnet);
        validators.push(pub_key);
        priv_keys.push(priv_key);
    }
    (validators, priv_keys)
}

pub fn make_signatures(
    redeem_script: &btc::RedeemScript,
    proposal: &AnchoringTx,
    inputs: &[u32],
    priv_keys: &[btc::PrivateKey],
) -> HashMap<u32, Vec<btc::Signature>> {
    let majority_count = (priv_keys.len() as u8) * 2 / 3 + 1;

    let mut signatures = inputs
        .iter()
        .map(|input| (*input, vec![None; priv_keys.len()]))
        .collect::<Vec<_>>();
    let mut priv_keys = priv_keys.iter().enumerate().collect::<Vec<_>>();
    rand::thread_rng().shuffle(&mut priv_keys);

    for (input_idx, input) in inputs.iter().enumerate() {
        let priv_keys_iter = priv_keys.iter().take(majority_count as usize);
        for &(id, priv_key) in priv_keys_iter {
            let sign = proposal.sign_input(redeem_script, *input, priv_key);
            signatures[input_idx].1[id] = Some(sign);
        }
    }

    signatures
        .iter()
        .map(|signs| {
            let input = signs.0;
            let signs = signs
                .1
                .iter()
                .filter_map(|x| x.clone())
                .take(majority_count as usize)
                .collect::<Vec<_>>();
            (input, signs)
        })
        .collect::<HashMap<_, _>>()
}

// Test key that extracted by `dumprpivkey` for address
// `cTvVLNQvaku9XG8LvKXEfWBvxehnj9S67FB3GZPP6mnY4c94AstC`
#[test]
fn test_privkey_serde_wif() {
    let privkey_str = "cTvVLNQvaku9XG8LvKXEfWBvxehnj9S67FB3GZPP6mnY4c94AstC";
    let privkey = btc::PrivateKey::from_base58check(privkey_str).unwrap();

    assert!(privkey.compressed);
    assert_eq!(privkey.network, Network::Testnet);
    assert_eq!(privkey.to_base58check(), privkey_str);
}

#[test]
fn test_txid_serde_hex() {
    let txid_hex = "0e4167aeb4769de5ad8d64d1b2342330c2b6aadc0ed9ad0d26ae8eafb18d9c87";
    let txid = btc::TxId::from_hex(txid_hex).unwrap();

    let json = serde_json::to_value(txid).unwrap().to_string();
    let txid2: btc::TxId = serde_json::from_str(&json).unwrap();
    assert_eq!(json, format!("\"{}\"", txid_hex));
    assert_eq!(txid2, txid);
}

#[test]
fn test_anchoring_txid() {
    let tx = AnchoringTx::from_hex(
        "010000000195a4472606ae658f1b9cbebd43f440def00c94341a3515024855\
         a1da8d80932800000000fd3d020047304402204e11d63db849f253095e1e0a400f2f0c01894083e97bfaef644b\
         1407b9fe5c4102207cc99ca986dfd99230e6641564d1f70009c5ec9a37da815c4e024c3ba837c0130148304502\
         2100d32536daa6e13989ebc7c908c27a0608517d5d967c8b6069dc047faa01e2a096022030f9c46738d9b701dd\
         944ce3e31af9898b9266460b2de6ff3319f2a8c51f7b430147304402206b8e4491e3b98861ba06cf64e78f425c\
         c553110535310f56f71dcd37de590b7f022051f0fa53cb74a1c73247224180cf026b61b7959d587ab6365dd19a\
         279d14cf45014830450221009fa024c767d8004eef882c6cffe9602f781c60d1a7c629d58576e3de41833a5b02\
         206d3b8dc86d052e112305e1fb32f61de77236f057523e22d58d82cbe37222e8fa01483045022100f1784c5e32\
         1fb2753fe725381d6f922d3f0edb94ff2eef52063f9c812489f61802202bec2903af6a5405db484ac73ab84470\
         7382f39a0b286a0453f2ed41d217c89e014ccf5521027b3e1c603ead09953bd0a8bd13a7a4830a144628996922\
         0b96515dd1745e06f521026b64f403914e43b7ebe9aa23017eb75eef1bc74469f8b1fa342e622565ab28db2103\
         503745e14331dac53528e666f1abab2c6b6e28767539a2827fe080bb475ec25021030a2ff505279a0e58cc3951\
         ada56bcf323955550d1b993c4cb1b7e94a672b31252102ebb5a22d5ec3c2bc36ab7e104553a89c69684a4dfb3c\
         8ea8fe2cb785c63425872102d9fea63c62d7cafcd4a3d20d77e06cf80cb25f3277ffce27d99c98f439323cee56\
         aeffffffff02000000000000000017a914ab6db56dbd716114594a0d3f072ec447f6d8fc698700000000000000\
         002c6a2a0128020000000000000062467691cf583d4fa78b18fafaf9801f505e0ef03baf0603fd4b0cd004cd1e\
         7500000000",
    ).unwrap();

    let txid_hex = "0e4167aeb4769de5ad8d64d1b2342330c2b6aadc0ed9ad0d26ae8eafb18d9c87";
    let txid = btc::TxId::from_hex(txid_hex).unwrap();
    let txid2 = tx.id();

    assert_eq!(txid2.be_hex_string(), txid_hex);
    assert_eq!(txid2, txid);
}

#[test]
fn test_anchoring_tx_serde() {
    let hex = "010000000148f4ae90d8c514a739f17dbbd405442171b09f1044183080b23b6557ce82c099010000000\
         0ffffffff0240899500000000001976a914b85133a96a5cadf6cddcfb1d17c79f42c3bbc9dd88ac00000000000\
         000002e6a2c6a2a6a28020000000000000062467691cf583d4fa78b18fafaf9801f505e0ef03baf0603fd4b0cd\
         004cd1e7500000000";
    let tx = AnchoringTx::from_hex(hex).unwrap();
    let json = serde_json::to_value(tx.clone()).unwrap().to_string();
    let tx2: AnchoringTx = serde_json::from_str(&json).unwrap();

    assert_eq!(tx2, tx);
}

#[test]
fn test_anchoring_tx_encoding_struct() {
    let hex = "010000000148f4ae90d8c514a739f17dbbd405442171b09f1044183080b23b6557ce82c099010000000\
         0ffffffff0240899500000000001976a914b85133a96a5cadf6cddcfb1d17c79f42c3bbc9dd88ac00000000000\
         000002e6a2c6a2a6a28020000000000000062467691cf583d4fa78b18fafaf9801f505e0ef03baf0603fd4b0cd\
         004cd1e7500000000";
    let tx = AnchoringTx::from_hex(hex).unwrap();
    let data = tx.clone().into_bytes();
    let tx2: AnchoringTx = AnchoringTx::from_bytes(data.into());

    assert_eq!(tx2, tx);
}

#[test]
fn test_anchoring_tx_message_field_rw_correct() {
    let hex = "010000000141d7585a6cb11e78c27fab0e8f8f8ede9285d6601fd4c4ab72cdadbb3b7af8030000000000\
         ffffffff02000000000000000017a914e084a290cf26998909b4fa5b42088918eeefee97870000000000\
         000000326a3045584f4e554d0100020000000000000062467691cf583d4fa78b18fafaf9801f505e0ef0\
         3baf0603fd4b0cd004cd1e7500000000";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    <AnchoringTx as Field>::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
    let dat2: Vec<u8> = unsafe { Field::read(&buf, 0, 8) };
    assert_eq!(dat2, dat);
}

#[test]
fn test_bitcoin_tx_message_field_rw_correct() {
    let hex = "010000000148f4ae90d8c514a739f17dbbd405442171b09f1044183080b23b6557ce82c099010000000\
         0ffffffff0240899500000000001976a914b85133a96a5cadf6cddcfb1d17c79f42c3bbc9dd88ac00000000000\
         000002e6a2c6a2a6a28020000000000000062467691cf583d4fa78b18fafaf9801f505e0ef03baf0603fd4b0cd\
         004cd1e7500000000";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    <BitcoinTx as Field>::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
    let dat2: Vec<u8> = unsafe { Field::read(&buf, 0, 8) };
    assert_eq!(dat2, dat);
}

#[should_panic(expected = "Result::unwrap()` on an `Err`")]
#[test]
fn test_anchoring_tx_message_field_rw_garbage_unwrap() {
    let hex = "00000000200000000000";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    let _: BitcoinTx = unsafe { Field::read(&buf, 0, 8) };
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err`")]
fn test_bitcoin_tx_message_field_rw_garbage_unwrap() {
    let hex = "000000000002000001";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    let _: BitcoinTx = unsafe { Field::read(&buf, 0, 8) };
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err`")]
fn test_anchoring_tx_message_field_rw_incorrect_check() {
    let hex = "000002000000000000";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    AnchoringTx::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err`")]
fn test_anchoring_tx_message_field_rw_without_payload_check() {
    let hex = "000000000000000000";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    AnchoringTx::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err`")]
fn test_anchoring_tx_message_field_rw_wrong_check() {
    // Correct non-anchoring tx, created by command:
    // `bitcoin-cli sendtoaddress "mynkNvvoysgzn3CX51KwyKyNVbEJEHs8Cw" 0.1`
    let hex = "02000000011b8ac5ff25dfe2b4675e86d77dda493ade980206ee6a7833729f07a2f1f49982000000004\
         84730440220620a9ea6cfe4f575d2edffa815705a50b95b3eec9e0259abe94a087fafebf59902200c4cd654a50\
         6137726bf608288539879d4ee939a3dc5bb8d4411bcbd2a0d836001feffffff0200d7e849000000001976a9146\
         18396019f30e77caaea0ec2d5ec5280e26ff23f88ac80969800000000001976a914c86ef8fb71b99cac9e5b1be\
         272ba5420656266f688ac58020000";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    AnchoringTx::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err`")]
fn test_funding_tx_message_field_rw_wrong_tx_kind_check() {
    // Correct non-funding tx, created by command:
    // `bitcoin-cli sendtoaddress "n4a3q23iUKZsmmrT5bVkeAsyqzvR5TmUbf" 0.0001` see transaction
    // b63170f59291c916b04fc65e110e4cbb7e835150ad1d62e6c03e929b832b4391 in the
    // https://www.blocktrail.com/tBTC
    let hex = "020000000197714d5c9db6334fc5043562a477abac3e4dae088fc94d68a7a634ec98b48373010000006\
         b483045022100a1a611cd455850681814b62cc138491f5e91b4e561ae38c7b26d6f5ba3253e4202203bc7aadc4\
         0452a5e1f76f025e198a7badf8374e476e51ab0baf5e1fe952d37cd012103231378cfe95565fe969e6a0fb6a70\
         2e2f97c8d48c395315c0f5075214aa19811feffffff0210270000000000001976a914fce0c2a6f0ff5d7ff9681\
         f861ca0b103a079c99088aca68f0f0c000000001976a9146fe0d927826f943309f1f9bd166a1888d757c08388a\
         c68121100";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    FundingTx::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err`")]
fn test_bitcoin_tx_message_field_rw_incorrect_check() {
    let hex = "000000000002000001";
    let dat = Vec::<u8>::from_hex(hex).unwrap();

    let mut buf = vec![255; 8];
    Field::write(&dat, &mut buf, 0, 8);

    BitcoinTx::check(&buf, 0.into(), 8.into(), 8.into()).unwrap();
}

#[test]
fn test_redeem_script_from_pubkeys() {
    let redeem_script_hex = "5321027db7837e51888e94c094703030d162c682c8dba312210f44ff440fbd5e5c247\
         32102bdd272891c9e4dfc3962b1fdffd5a59732019816f9db4833634dbdaf01a401a52103280883dc31ccaee34\
         218819aaa245480c35a33acd91283586ff6d1284ed681e52103e2bc790a6e32bf5a766919ff55b1f9e9914e13a\
         ed84f502c0e4171976e19deb054ae";
    let keys = [
        "027db7837e51888e94c094703030d162c682c8dba312210f44ff440fbd5e5c2473",
        "02bdd272891c9e4dfc3962b1fdffd5a59732019816f9db4833634dbdaf01a401a5",
        "03280883dc31ccaee34218819aaa245480c35a33acd91283586ff6d1284ed681e5",
        "03e2bc790a6e32bf5a766919ff55b1f9e9914e13aed84f502c0e4171976e19deb0",
    ].into_iter()
        .map(|x| btc::PublicKey::from_hex(x).unwrap())
        .collect::<Vec<_>>();

    let redeem_script = btc::RedeemScript::from_pubkeys(&keys, 3);
    assert_eq!(redeem_script.to_hex(), redeem_script_hex);
    assert_eq!(
        redeem_script.to_address(Network::Testnet).to_string(),
        "2N1mHzwKTmjnC7JjqeGFBRKYE4WDTjTfop1"
    );
    assert_eq!(
        btc::RedeemScript::from_hex(redeem_script_hex).unwrap(),
        redeem_script
    );

    let compressed_redeem_script = redeem_script.compressed(Network::Testnet);
    assert_eq!(
        compressed_redeem_script.to_hex(),
        "5321027db7837e51888e94c094703030d162c682c8dba312210f44ff440fbd5e5c24732102bdd27289\
         1c9e4dfc3962b1fdffd5a59732019816f9db4833634dbdaf01a401a52103280883dc31ccaee34218819\
         aaa245480c35a33acd91283586ff6d1284ed681e52103e2bc790a6e32bf5a766919ff55b1f9e9914e13\
         aed84f502c0e4171976e19deb054ae"
    );
    assert_eq!(
        compressed_redeem_script.compressed(Network::Testnet),
        compressed_redeem_script
    );
}

#[test]
fn test_sign_raw_transaction() {
    let unsigned_tx = BitcoinTx::from_hex(
        "01000000015d1b8ba33a162d8f6e7c5707fbb557e726c32f30f77f2\
         ba348a48c3c5d71ee0b0000000000ffffffff02b80b00000000000017a914889fc9c82819c7a728974ffa78cc8\
         84e3e9e68838700000000000000002c6a2a6a28020000000000000062467691cf583d4fa78b18fafaf9801f505\
         e0ef03baf0603fd4b0cd004cd1e7500000000",
    ).unwrap();

    let priv_key = RawPrivateKey::from_base58check(
        "cVC9eJN5peJemWn1byyWcWDevg6xLNXtACjHJWmrR5ynsCu8mkQE",
    ).unwrap();
    let pub_key = {
        let context = Secp256k1::new();
        RawPublicKey::from_secret_key(&context, priv_key.secret_key()).unwrap()
    };

    let redeem_script = btc::RedeemScript::from_hex(
        "5321027db7837e51888e94c094703030d162c682c8dba\
         312210f44ff440fbd5e5c24732102bdd272891c9e4dfc3962b1fdffd5a59732019816f9db4833634dbdaf01a40\
         1a52103280883dc31ccaee34218819aaa245480c35a33acd91283586ff6d1284ed681e52103e2bc790a6e32bf5\
         a766919ff55b1f9e9914e13aed84f502c0e4171976e19deb054ae",
    ).unwrap();
    let mut actual_signature =
        sign_tx_input(&unsigned_tx, 0, &redeem_script, priv_key.secret_key());
    actual_signature.push(SigHashType::All.as_u32() as u8);

    assert_eq!(
        encode_hex(&actual_signature),
        "304502210092f1fd6367677ef63dfddfb69cb3644ab10a7c497e5cd391e1d36284dca6a570022021dc\
         2132349afafb9273600698d806f6d5f55756fcc058fba4e49c066116124e01"
    );
    assert!(verify_tx_input(
        &unsigned_tx,
        0,
        &redeem_script,
        &pub_key,
        &actual_signature[0..actual_signature.len() - 1],
    ));
}

#[test]
fn test_redeem_script_pubkey() {
    let redeem_script = btc::RedeemScript::from_hex(
        "55210351d8beec8ef4faef9a299640f2f2c8427b4c5ec\
         655da3bdf9c78bb02debce7052103c39016fa9182f84d367d382b561a3db2154041926e4e461607a903ce2b78d\
         bf72103cba17beba839abbc377f8ff8a908199d544ef821509a45ec3b5684e733e4d73b2102014c953a69d452a\
         8c385d1c68e985d697d04f79bf0ddb11e2852e40b9bb880a4210389cbc7829f40deff4acef55babf7dc486a805\
         ad0f4533e665dee4dd6d38157a32103c60e0aeb3d87b05f49341aa88a347237ab2cff3e91a78d23880080d05f8\
         f08e756ae",
    ).unwrap();

    assert_eq!(
        redeem_script.script_pubkey(btc::Network::Testnet).to_hex(),
        "a914544fa2db1f36b091bbee603c0bc7675fe34655ff87"
    );
}

#[test]
fn test_anchoring_tx_sign() {
    let _ = helpers::init_logger();

    let priv_keys = [
        "cVC9eJN5peJemWn1byyWcWDevg6xLNXtACjHJWmrR5ynsCu8mkQE",
        "cMk66oMazTgquBVaBLHzDi8FMgAaRN3tSf6iZykf9bCh3D3FsLX1",
        "cT2S5KgUQJ41G6RnakJ2XcofvoxK68L9B44hfFTnH4ddygaxi7rc",
        "cRUKB8Nrhxwd5Rh6rcX3QK1h7FosYPw5uzEsuPpzLcDNErZCzSaj",
    ].iter()
        .map(|x| btc::PrivateKey::from_base58check(x).unwrap())
        .collect::<Vec<_>>();

    let pub_keys = [
        "03475ab0e9cfc6015927e662f6f8f088de12287cee1a3237aeb497d1763064690c",
        "02a63948315dda66506faf4fecd54b085c08b13932a210fa5806e3691c69819aa0",
        "0230cb2805476bf984d2236b56ff5da548dfe116daf2982608d898d9ecb3dceb49",
        "036e4777c8d19ccaa67334491e777f221d37fd85d5786a4e5214b281cf0133d65e",
    ].iter()
        .map(|x| btc::PublicKey::from_hex(x).unwrap())
        .collect::<Vec<_>>();
    let redeem_script = btc::RedeemScript::from_pubkeys(pub_keys.iter(), 3)
        .compressed(Network::Testnet);

    let prev_tx = AnchoringTx::from_hex(
        "01000000014970bd8d76edf52886f62e3073714bddc6c33bccebb6b1d\
         06db8c87fb1103ba000000000fd670100483045022100e6ef3de83437c8dc33a8099394b7434dfb40c73631fc4\
         b0378bd6fb98d8f42b002205635b265f2bfaa6efc5553a2b9e98c2eabdfad8e8de6cdb5d0d74e37f1e19852014\
         7304402203bb845566633b726e41322743677694c42b37a1a9953c5b0b44864d9b9205ca10220651b701271987\
         1c36d0f89538304d3f358da12b02dab2b4d74f2981c8177b69601473044022052ad0d6c56aa6e971708f079073\
         260856481aeee6a48b231bc07f43d6b02c77002203a957608e4fbb42b239dd99db4e243776cc55ed8644af21fa\
         80fd9be77a59a60014c8b532103475ab0e9cfc6015927e662f6f8f088de12287cee1a3237aeb497d1763064690\
         c2102a63948315dda66506faf4fecd54b085c08b13932a210fa5806e3691c69819aa0210230cb2805476bf984d\
         2236b56ff5da548dfe116daf2982608d898d9ecb3dceb4921036e4777c8d19ccaa67334491e777f221d37fd85d\
         5786a4e5214b281cf0133d65e54aeffffffff02b80b00000000000017a914bff50e89fa259d83f78f2e796f572\
         83ca10d6e678700000000000000002c6a2a01280000000000000000f1cb806d27e367f1cac835c22c8cc24c402\
         a019e2d3ea82f7f841c308d830a9600000000",
    ).unwrap();
    let funding_tx = FundingTx::from_hex(
        "01000000019532a4022a22226a6f694c3f21216b2c9f5c1c79007eb7\
         d3be06bc2f1f9e52fb000000006a47304402203661efd05ca422fad958b534dbad2e1c7db42bbd1e73e9b91f43\
         a2f7be2f92040220740cf883273978358f25ca5dd5700cce5e65f4f0a0be2e1a1e19a8f168095400012102ae1b\
         03b0f596be41a247080437a50f4d8e825b170770dcb4e5443a2eb2ecab2afeffffff02a00f00000000000017a9\
         14bff50e89fa259d83f78f2e796f57283ca10d6e678716e1ff05000000001976a91402f5d7475a10a9c24cea32\
         575bd8993d3fabbfd388ac089e1000",
    ).unwrap();

    let tx = TransactionBuilder::with_prev_tx(&prev_tx, 0)
        .add_funds(&funding_tx, 0)
        .payload(
            Height(10),
            Hash::from_hex(
                "164d236bbdb766e64cec57847e3a0509d4fc77fa9c17b7e61e48f7a3eaa8dbc9",
            ).unwrap(),
        )
        .fee(1000)
        .send_to(btc::Address::from_script(&redeem_script, Network::Testnet))
        .into_transaction()
        .unwrap();

    let mut signatures = HashMap::new();
    for input in tx.inputs() {
        let mut input_signs = Vec::new();
        for priv_key in &priv_keys {
            let sign = tx.sign_input(&redeem_script, input, priv_key);
            input_signs.push(sign);
        }
        signatures.insert(input, input_signs);
    }

    for (input, signs) in &signatures {
        for (id, signature) in signs.iter().enumerate() {
            assert!(tx.verify_input(
                &redeem_script,
                *input,
                &pub_keys[id],
                signature.as_ref(),
            ));
        }
    }
}

#[test]
fn test_anchoring_tx_output_address() {
    let tx = AnchoringTx::from_hex(
        "01000000014970bd8d76edf52886f62e3073714bddc6c33bccebb6b1d06db8\
         c87fb1103ba000000000fd670100483045022100e6ef3de83437c8dc33a8099394b7434dfb40c73631fc4b0378\
         bd6fb98d8f42b002205635b265f2bfaa6efc5553a2b9e98c2eabdfad8e8de6cdb5d0d74e37f1e1985201473044\
         02203bb845566633b726e41322743677694c42b37a1a9953c5b0b44864d9b9205ca10220651b7012719871c36d\
         0f89538304d3f358da12b02dab2b4d74f2981c8177b69601473044022052ad0d6c56aa6e971708f07907326085\
         6481aeee6a48b231bc07f43d6b02c77002203a957608e4fbb42b239dd99db4e243776cc55ed8644af21fa80fd9\
         be77a59a60014c8b532103475ab0e9cfc6015927e662f6f8f088de12287cee1a3237aeb497d1763064690c2102\
         a63948315dda66506faf4fecd54b085c08b13932a210fa5806e3691c69819aa0210230cb2805476bf984d2236b\
         56ff5da548dfe116daf2982608d898d9ecb3dceb4921036e4777c8d19ccaa67334491e777f221d37fd85d5786a\
         4e5214b281cf0133d65e54aeffffffff02b80b00000000000017a914bff50e89fa259d83f78f2e796f57283ca1\
         0d6e678700000000000000002c6a2a01280000000000000000f1cb806d27e367f1cac835c22c8cc24c402a019e\
         2d3ea82f7f841c308d830a9600000000",
    ).unwrap();

    let pub_keys = [
        "03475ab0e9cfc6015927e662f6f8f088de12287cee1a3237aeb497d1763064690c",
        "02a63948315dda66506faf4fecd54b085c08b13932a210fa5806e3691c69819aa0",
        "0230cb2805476bf984d2236b56ff5da548dfe116daf2982608d898d9ecb3dceb49",
        "036e4777c8d19ccaa67334491e777f221d37fd85d5786a4e5214b281cf0133d65e",
    ].iter()
        .map(|x| btc::PublicKey::from_hex(x).unwrap())
        .collect::<Vec<_>>();
    let redeem_script = btc::RedeemScript::from_pubkeys(&pub_keys, 3).compressed(Network::Testnet);

    assert_eq!(
        tx.output_address(Network::Testnet),
        redeem_script.to_address(Network::Testnet)
    );
}

#[test]
fn test_anchoring_tx_prev_chain() {
    let prev_tx = AnchoringTx::from_hex(
        "01000000014970bd8d76edf52886f62e3073714bddc6c33bccebb6b1d\
         06db8c87fb1103ba000000000fd670100483045022100e6ef3de83437c8dc33a8099394b7434dfb40c73631fc4\
         b0378bd6fb98d8f42b002205635b265f2bfaa6efc5553a2b9e98c2eabdfad8e8de6cdb5d0d74e37f1e19852014\
         7304402203bb845566633b726e41322743677694c42b37a1a9953c5b0b44864d9b9205ca10220651b701271987\
         1c36d0f89538304d3f358da12b02dab2b4d74f2981c8177b69601473044022052ad0d6c56aa6e971708f079073\
         260856481aeee6a48b231bc07f43d6b02c77002203a957608e4fbb42b239dd99db4e243776cc55ed8644af21fa\
         80fd9be77a59a60014c8b532103475ab0e9cfc6015927e662f6f8f088de12287cee1a3237aeb497d1763064690\
         c2102a63948315dda66506faf4fecd54b085c08b13932a210fa5806e3691c69819aa0210230cb2805476bf984d\
         2236b56ff5da548dfe116daf2982608d898d9ecb3dceb4921036e4777c8d19ccaa67334491e777f221d37fd85d\
         5786a4e5214b281cf0133d65e54aeffffffff02b80b00000000000017a914bff50e89fa259d83f78f2e796f572\
         83ca10d6e678700000000000000002c6a2a01280000000000000000f1cb806d27e367f1cac835c22c8cc24c402\
         a019e2d3ea82f7f841c308d830a9600000000",
    ).unwrap();
    let tx = TransactionBuilder::with_prev_tx(&prev_tx, 0)
        .fee(1000)
        .payload(Height::zero(), Hash::default())
        .prev_tx_chain(Some(prev_tx.id()))
        .send_to(
            btc::Address::from_base58check("2N1mHzwKTmjnC7JjqeGFBRKYE4WDTjTfop1").unwrap(),
        )
        .into_transaction()
        .unwrap();


    assert_eq!(tx.payload().prev_tx_chain, Some(prev_tx.id()));
}

#[test]
fn test_tx_kind_funding() {
    let tx = BitcoinTx::from_hex(
        "01000000019532a4022a22226a6f694c3f21216b2c9f5c1c79007eb7d3be06bc\
         2f1f9e52fb000000006a47304402203661efd05ca422fad958b534dbad2e1c7db42bbd1e73e9b91f43a2f7be2f\
         92040220740cf883273978358f25ca5dd5700cce5e65f4f0a0be2e1a1e19a8f168095400012102ae1b03b0f596\
         be41a247080437a50f4d8e825b170770dcb4e5443a2eb2ecab2afeffffff02a00f00000000000017a914bff50e\
         89fa259d83f78f2e796f57283ca10d6e678716e1ff05000000001976a91402f5d7475a10a9c24cea32575bd899\
         3d3fabbfd388ac089e1000",
    ).unwrap();
    match TxKind::from(tx) {
        TxKind::FundingTx(_) => {}
        _ => panic!("Wrong tx kind!"),
    }
}

#[test]
fn test_tx_kind_anchoring() {
    let tx = BitcoinTx::from_hex(
        "010000000141d7585a6cb11e78c27fab0e8f8f8ede9285d6601fd4c4ab72cdadb\
         b3b7af8030000000000ffffffff02000000000000000017a914e084a290cf2699\
         8909b4fa5b42088918eeefee97870000000000000000326a3045584f4e554d010\
         0020000000000000062467691cf583d4fa78b18fafaf9801f505e0ef03baf0603\
         fd4b0cd004cd1e7500000000",
    ).unwrap();
    match TxKind::from(tx) {
        TxKind::Anchoring(_) => {}
        _ => panic!("Wrong tx kind!"),
    }
}

#[test]
fn test_tx_kind_other() {
    let tx = BitcoinTx::from_hex(
        "0100000001cea827387bc0bb1b5e6afa6e6d557123e4432e47bad8c2d94214a9\
         cd1e2e074b010000006a473044022034d463312dd75445ad078b1159a75c0b148388b36686b69da8aecca863e6\
         3dc3022071ef86a064bd15f11ec89059072bbd3e3d3bb6c5e9b10712e0e2dc6710520bb00121035e63a48d3425\
         0dbbcc58fdc0ab63b901769e71035e19e0eee1a87d433a96723afeffffff0296a6f80b000000001976a914b5d7\
         055cfdacc803e5547b981faa693c5aaa813b88aca0860100000000001976a914f5548cb02bb197f071934a0ea3\
         eeb5878cb59dff88ac03a21000",
    ).unwrap();
    match TxKind::from(tx) {
        TxKind::Other(_) => {}
        _ => panic!("Wrong tx kind!"),
    }
}

#[test]
fn test_tx_verify_sighash_type_correct() {
    let (pub_keys, priv_keys) = gen_anchoring_keys(4);
    let redeem_script = btc::RedeemScript::from_pubkeys(&pub_keys, 3).compressed(Network::Testnet);

    let tx = dummy_anchoring_tx(&redeem_script);
    let pub_key = &pub_keys[0];
    let btc_signature = tx.sign_input(&redeem_script, 0, &priv_keys[0]);

    assert_eq!(
        *btc_signature.last().unwrap(),
        SigHashType::All.as_u32() as u8
    );
    assert!(tx.verify_input(&redeem_script, 0, &pub_key, &btc_signature));
}

#[test]
fn test_tx_verify_incorrect_signature() {
    let (pub_keys, priv_keys) = gen_anchoring_keys(4);
    let redeem_script = btc::RedeemScript::from_pubkeys(&pub_keys, 3).compressed(Network::Testnet);

    let tx = dummy_anchoring_tx(&redeem_script);
    let pub_key = &pub_keys[0];
    let mut btc_signature = tx.sign_input(&redeem_script, 0, &priv_keys[0]);
    btc_signature[8] = btc_signature[8].wrapping_add(63);

    assert!(!tx.verify_input(
        &redeem_script,
        0,
        &pub_key,
        &btc_signature,
    ));
}

/// Verifies that non-strict DER signatures do not pass verification
/// See https://github.com/bitcoin/bips/blob/master/bip-0066.mediawiki
#[test]
fn test_tx_verify_non_strict_der_signature() {
    let (pub_keys, priv_keys) = gen_anchoring_keys(4);
    let redeem_script = btc::RedeemScript::from_pubkeys(&pub_keys, 3).compressed(Network::Testnet);

    let tx = dummy_anchoring_tx(&redeem_script);
    let pub_key = &pub_keys[0];

    let btc_signature_1 = tx.sign_input(&redeem_script, 0, &priv_keys[0]);
    let mut btc_signature_2 = btc_signature_1.clone();
    // Set an incorrect length of the DER-encoded sequence in the signature
    btc_signature_2[1] = btc_signature_2[1].wrapping_add(1);

    assert!(btc_signature_1 != btc_signature_2);
    assert!(tx.verify_input(
        &redeem_script,
        0,
        &pub_key,
        &btc_signature_1,
    ));
    assert!(!tx.verify_input(
        &redeem_script,
        0,
        &pub_key,
        &btc_signature_2,
    ));
}

#[test]
fn test_tx_verify_sighash_type_wrong() {
    let (pub_keys, priv_keys) = gen_anchoring_keys(4);
    let redeem_script = btc::RedeemScript::from_pubkeys(&pub_keys, 3).compressed(Network::Testnet);

    let tx = dummy_anchoring_tx(&redeem_script);
    let pub_key = &pub_keys[0];
    let mut btc_signature = tx.sign_input(&redeem_script, 0, &priv_keys[0]);
    *btc_signature.last_mut().unwrap() = SigHashType::Single.as_u32() as u8;

    assert!(tx.verify_input(&redeem_script, 0, &pub_key, &btc_signature));
}

// rpc tests. Works through `rpc` by given env variables.
// See the `anchoring_client` method on top of this file.
#[cfg(feature = "rpc_tests")]
mod rpc {
    use super::*;

    use bitcoin::network::constants::Network;
    use bitcoin::util::base58::ToBase58;
    use bitcoinrpc;

    use exonum::helpers::{self, Height};
    use exonum::crypto::{hash, Hash};

    use details::rpc::{AnchoringRpcConfig, BitcoinRelay, RpcClient};
    use details::btc::transactions::{AnchoringTx, FundingTx, TransactionBuilder};
    use details::btc;

    fn anchoring_client() -> RpcClient {
        use std::env;
        let rpc = AnchoringRpcConfig {
            host: env::var("ANCHORING_RELAY_HOST")
                .expect("Env variable ANCHORING_RELAY_HOST needs to be setted")
                .parse()
                .unwrap(),
            username: env::var("ANCHORING_USER").ok(),
            password: env::var("ANCHORING_PASSWORD").ok(),
        };

        RpcClient::from(rpc)
    }

    pub fn create_multisig_address<'a, I>(
        client: &BitcoinRelay,
        network: btc::Network,
        count: u8,
        pub_keys: I,
    ) -> Result<(btc::RedeemScript, btc::Address), bitcoinrpc::Error>
    where
        I: IntoIterator<Item = &'a btc::PublicKey>,
    {
        let redeem_script = btc::RedeemScript::from_pubkeys(pub_keys, count).compressed(network);
        let addr = btc::Address::from_script(&redeem_script, network);

        client.watch_address(&addr, false)?;
        Ok((redeem_script, addr))
    }

    fn send_anchoring_tx(
        client: &BitcoinRelay,
        redeem_script: &btc::RedeemScript,
        to: &btc::Address,
        block_height: Height,
        block_hash: Hash,
        priv_keys: &[btc::PrivateKey],
        anchoring_tx: AnchoringTx,
        additional_funds: &[FundingTx],
        fee: u64,
    ) -> AnchoringTx {
        let tx = {
            let mut builder = TransactionBuilder::with_prev_tx(&anchoring_tx, 0)
                .fee(fee)
                .payload(block_height, block_hash)
                .send_to(to.clone());
            for funding_tx in additional_funds {
                let out = funding_tx.find_out(to).unwrap();
                builder = builder.add_funds(funding_tx, out);
            }
            builder.into_transaction().unwrap()
        };
        trace!("Proposal anchoring_tx={:#?}, txid={}", tx, tx.txid());

        let inputs = tx.inputs().collect::<Vec<_>>();
        let signatures = make_signatures(redeem_script, &tx, inputs.as_slice(), priv_keys);
        let tx = tx.finalize(redeem_script, signatures);
        client.send_transaction(tx.clone().into()).unwrap();

        let payload = tx.payload();
        assert_eq!(payload.block_height, block_height);
        assert_eq!(payload.block_hash, block_hash);

        trace!("Sended anchoring_tx={:#?}, txid={}", tx, tx.txid());
        let unspent_transactions = client.unspent_transactions(to).unwrap();
        let lect_tx = &unspent_transactions[0];
        assert_eq!(lect_tx.body.0, tx.0);
        tx
    }

    #[test]
    fn test_rpc_nonexistent_transaction_get_info() {
        let _ = helpers::init_logger();

        let client = anchoring_client();

        let txid = btc::TxId::from_hex(
            "21972c3e2b7047c41c0ece2f18223775e62a24822923c846b3a7cabfd8585d73",
        ).unwrap();
        assert!(client.get_transaction_info(txid).unwrap().is_none());
        assert!(client.get_transaction(txid).unwrap().is_none());
    }

    #[test]
    fn test_rpc_unspent_funding_tx() {
        let _ = helpers::init_logger();

        let client = anchoring_client();

        let (validators, _) = gen_anchoring_keys(4);

        let majority_count = ::majority_count(4);
        let (_, address) =
            create_multisig_address(&client, Network::Testnet, majority_count, validators.iter())
                .unwrap();

        let funding_tx = client.send_to_address(&address, 1000).unwrap();
        let info = funding_tx.has_unspent_info(&client, &address).unwrap();
        assert!(info.is_some());
        trace!("{:#?}", info);
    }

    #[test]
    fn test_rpc_anchoring_tx_chain() {
        let _ = helpers::init_logger();

        let client = anchoring_client();

        let (validators, priv_keys) = gen_anchoring_keys(4);
        let majority_count = ::majority_count(4);
        let (redeem_script, addr) =
            create_multisig_address(&client, Network::Testnet, majority_count, validators.iter())
                .unwrap();
        trace!("multisig_address={:#?}", redeem_script);

        let fee = 1000;
        let block_height = Height(2);
        let block_hash = hash(&[1, 3, 5]);

        // Make anchoring txs chain
        let total_funds = 4000;
        let mut utxo_tx = {
            let funding_tx = client.send_to_address(&addr, total_funds).unwrap();
            let out = funding_tx.find_out(&addr).unwrap();
            trace!("funding_tx={:#?}", funding_tx);

            let tx = TransactionBuilder::with_prev_tx(&funding_tx, out)
                .payload(block_height, block_hash)
                .send_to(addr.clone())
                .fee(fee)
                .prev_tx_chain(Some(funding_tx.id()))
                .into_transaction()
                .unwrap();
            trace!("Proposal anchoring_tx={:#?}, txid={}", tx, tx.txid());

            let signatures = make_signatures(&redeem_script, &tx, &[0], &priv_keys);
            let tx = tx.finalize(&redeem_script, signatures);
            client.send_transaction(tx.clone().into()).unwrap();
            trace!("Sended anchoring_tx={:#?}, txid={}", tx, tx.txid());

            assert!(
                funding_tx
                    .has_unspent_info(&client, &addr)
                    .unwrap()
                    .is_none()
            );
            let lect_tx = client
                .unspent_transactions(&addr)
                .unwrap()
                .first()
                .unwrap()
                .clone();
            assert_eq!(lect_tx.body.0, tx.0);
            tx
        };

        let utxos = client
            .listunspent(0, 9999999, &[addr.to_base58check()])
            .unwrap();
        trace!("utxos={:#?}", utxos);

        // Send anchoring txs
        let mut out_funds = utxo_tx.amount();
        trace!("out_funds={}", out_funds);
        while out_funds >= fee {
            utxo_tx = send_anchoring_tx(
                &client,
                &redeem_script,
                &addr,
                block_height,
                block_hash,
                &priv_keys,
                utxo_tx,
                &[],
                fee,
            );

            let payload = utxo_tx.payload();
            assert_eq!(payload.block_height, block_height);
            assert_eq!(payload.block_hash, block_hash);
            out_funds -= fee;
        }

        // Try to add funding input
        let funding_tx = client.send_to_address(&addr, fee * 3).unwrap();
        utxo_tx = send_anchoring_tx(
            &client,
            &redeem_script,
            &addr,
            block_height,
            block_hash,
            &priv_keys,
            utxo_tx,
            &[funding_tx],
            fee,
        );

        // Send to next addr
        let (validators2, priv_keys2) = gen_anchoring_keys(6);
        let majority_count2 = ::majority_count(6);
        let (redeem_script2, addr2) = create_multisig_address(
            &client,
            Network::Testnet,
            majority_count2,
            validators2.iter(),
        ).unwrap();

        trace!("new_multisig_address={:#?}", redeem_script2);
        utxo_tx = send_anchoring_tx(
            &client,
            &redeem_script,
            &addr2,
            block_height,
            block_hash,
            &priv_keys,
            utxo_tx,
            &[],
            fee,
        );

        send_anchoring_tx(
            &client,
            &redeem_script2,
            &addr2,
            block_height,
            block_hash,
            &priv_keys2,
            utxo_tx,
            &[],
            fee,
        );
    }

    #[test]
    #[should_panic(expected = "InsufficientFunds")]
    fn test_rpc_anchoring_tx_chain_insufficient_funds() {
        let _ = helpers::init_logger();

        let client = anchoring_client();

        let (validators, priv_keys) = gen_anchoring_keys(4);
        let majority_count = ::majority_count(4);
        let (redeem_script, addr) =
            create_multisig_address(&client, Network::Testnet, majority_count, validators.iter())
                .unwrap();
        trace!("multisig_address={:#?}", redeem_script);

        let fee = 1000;
        let block_height = Height(2);
        let block_hash = hash(&[1, 3, 5]);

        // Make anchoring txs chain
        let total_funds = 4000;
        let mut utxo_tx = {
            let funding_tx = client.send_to_address(&addr, total_funds).unwrap();
            let out = funding_tx.find_out(&addr).unwrap();
            trace!("funding_tx={:#?}", funding_tx);

            let tx = TransactionBuilder::with_prev_tx(&funding_tx, out)
                .payload(block_height, block_hash)
                .send_to(addr.clone())
                .fee(fee)
                .into_transaction()
                .unwrap();
            trace!("Proposal anchoring_tx={:#?}, txid={}", tx, tx.txid());

            let signatures = make_signatures(&redeem_script, &tx, &[0], &priv_keys);
            let tx = tx.finalize(&redeem_script, signatures);
            client.send_transaction(tx.clone().into()).unwrap();
            trace!("Sended anchoring_tx={:#?}, txid={}", tx, tx.txid());

            assert!(
                funding_tx
                    .has_unspent_info(&client, &addr)
                    .unwrap()
                    .is_none()
            );
            let lect_tx = client
                .unspent_transactions(&addr)
                .unwrap()
                .first()
                .unwrap()
                .clone();
            assert_eq!(lect_tx.body, tx.0);
            tx
        };

        let utxos = client
            .listunspent(0, 9999999, &[addr.to_base58check()])
            .unwrap();
        trace!("utxos={:#?}", utxos);

        // Send anchoring txs
        let mut out_funds = utxo_tx.amount();
        trace!("out_funds={}", out_funds);
        while out_funds >= fee {
            utxo_tx = send_anchoring_tx(
                &client,
                &redeem_script,
                &addr,
                block_height,
                block_hash,
                &priv_keys,
                utxo_tx,
                &[],
                fee,
            );

            let payload = utxo_tx.payload();
            assert_eq!(payload.block_height, block_height);
            assert_eq!(payload.block_hash, block_hash);
            out_funds -= fee;
        }

        // Try to send tx without funds
        send_anchoring_tx(
            &client,
            &redeem_script,
            &addr,
            block_height,
            block_hash,
            &priv_keys,
            utxo_tx,
            &[],
            fee,
        );
    }
}
