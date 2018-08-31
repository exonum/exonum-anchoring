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

use std::mem;

use bitcoin::blockdata::transaction::SigHashType;
use btc_transaction_utils::{multisig::RedeemScript, p2wsh, TxInRef};
use byteorder::{ByteOrder, LittleEndian};
use libc::c_void;
use secp256k1::ffi;
use secp256k1::key;
use secp256k1::key::SecretKey;
use secp256k1::Error;
use secp256k1::{Message, Signature};
use secp256k1::{Secp256k1, SignOnly};

use exonum_btc_anchoring::details::btc::transactions::RawBitcoinTx;

/// The structure with the same memory representation as the `secp256k1::Secp256k1`.
#[derive(Clone, Copy)]
pub struct Context {
    ctx: *mut ffi::Context,
}

impl Context {
    /// Same as the 'secp256k1::Secp256k1::sign` but has a nonce argument.
    pub fn sign(&self, msg: &Message, sk: &key::SecretKey, nonce: u64) -> Result<Signature, Error> {
        let nonce_array = {
            let mut data = [0; 32];
            LittleEndian::write_u64(&mut data, nonce);
            data
        };

        let mut ret = unsafe { ffi::Signature::blank() };
        unsafe {
            // We can assume the return value because it's not possible to construct
            // an invalid signature from a valid `Message` and `SecretKey`
            assert_eq!(
                ffi::secp256k1_ecdsa_sign(
                    self.ctx,
                    &mut ret,
                    msg.as_ptr(),
                    sk.as_ptr(),
                    ffi::secp256k1_nonce_function_rfc6979,
                    nonce_array.as_ptr() as *const c_void,
                ),
                1
            );
        }
        Ok(Signature::from(ret))
    }
}

fn get_ffi_context(ctx: &mut Secp256k1<SignOnly>) -> Context {
    unsafe {
        let ctx_ptr: *mut Context = mem::transmute(ctx as *mut Secp256k1<SignOnly>);
        *ctx_ptr
    }
}

fn sign_with_nonce(
    ctx: &mut Secp256k1<SignOnly>,
    msg: &Message,
    sk: &key::SecretKey,
    nonce: u64,
) -> Result<Signature, Error> {
    let ctx = get_ffi_context(ctx);
    ctx.sign(msg, sk, nonce)
}

pub fn sign_tx_input_with_nonce(
    tx: &RawBitcoinTx,
    input: usize,
    subscript: &RedeemScript,
    prev_tx: &RawBitcoinTx,
    sec_key: &SecretKey,
    nonce: u64,
) -> Vec<u8> {
    let sighash = {
        let mut signer = p2wsh::InputSigner::new(subscript.clone());
        signer.signature_hash(TxInRef::new(tx, input), prev_tx)
    };
    // Make signature
    let mut context = Secp256k1::signing_only();
    let msg = Message::from_slice(&sighash[..]).unwrap();
    let mut sign = sign_with_nonce(&mut context, &msg, sec_key, nonce)
        .unwrap()
        .serialize_der(&context);
    sign.push(SigHashType::All as u8);
    sign
}
