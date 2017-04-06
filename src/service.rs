use std::sync::{Arc, Mutex};

use bitcoin::util::base58::ToBase58;
use serde_json::value::{ToJson, Value};
use rand::{thread_rng, Rng};

use exonum::blockchain::{Service, Transaction, NodeState};
use exonum::crypto::Hash;
use exonum::messages::{RawTransaction, FromRaw, Error as MessageError};
use exonum::storage::{View, Error as StorageError};

use details::btc;
use details::rpc::{AnchoringRpc, AnchoringRpcConfig};
use details::transactions::FundingTx;
use local_storage::AnchoringNodeConfig;
use handler::AnchoringHandler;
use blockchain::consensus_storage::AnchoringConfig;
use blockchain::schema::AnchoringSchema;
use blockchain::dto::AnchoringMessage;
use error::Error as ServiceError;

pub use blockchain::ANCHORING_SERVICE_ID;

/// An anchoring service implementation for `Exonum` blockchain.
pub struct AnchoringService {
    genesis: AnchoringConfig,
    handler: Arc<Mutex<AnchoringHandler>>,
}

impl AnchoringService {
    pub fn new(client: AnchoringRpc,
               genesis: AnchoringConfig,
               cfg: AnchoringNodeConfig)
               -> AnchoringService {
        AnchoringService {
            genesis: genesis,
            handler: Arc::new(Mutex::new(AnchoringHandler::new(client, cfg))),
        }
    }

    /// Returns an internal handler
    pub fn handler(&self) -> Arc<Mutex<AnchoringHandler>> {
        self.handler.clone()
    }
}

impl Service for AnchoringService {
    fn service_id(&self) -> u16 {
        ANCHORING_SERVICE_ID
    }

    fn state_hash(&self, view: &View) -> Result<Vec<Hash>, StorageError> {
        AnchoringSchema::new(view).state_hash()
    }

    fn tx_from_raw(&self, raw: RawTransaction) -> Result<Box<Transaction>, MessageError> {
        AnchoringMessage::from_raw(raw).map(|tx| Box::new(tx) as Box<Transaction>)
    }

    fn handle_genesis_block(&self, view: &View) -> Result<Value, StorageError> {
        let handler = self.handler.lock().unwrap();
        let cfg = self.genesis.clone();
        let (_, addr) = cfg.redeem_script();
        handler
            .client
            .importaddress(&addr.to_base58check(), "multisig", false, false)
            .unwrap();

        AnchoringSchema::new(view).create_genesis_config(&cfg)?;
        Ok(cfg.to_json())
    }

    fn handle_commit(&self, state: &mut NodeState) -> Result<(), StorageError> {
        match self.handler.lock().unwrap().handle_commit(state) {
            Err(ServiceError::Storage(e)) => Err(e),
            Err(e) => {
                error!("An error occured: {:?}", e);
                Ok(())
            }
            Ok(()) => Ok(()),
        }
    }
}


/// Generates testnet configuration by given rpc for given given nodes amount
/// using given random number generator.
///
/// Note: Bitcoin node that used by rpc have to enough bitcoin amount to generate
/// funding transaction by given `total_funds`.
pub fn gen_anchoring_testnet_config_with_rng<R>(client: &AnchoringRpc,
                                                network: btc::Network,
                                                count: u8,
                                                total_funds: u64,
                                                rng: &mut R)
                                                -> (AnchoringConfig, Vec<AnchoringNodeConfig>)
    where R: Rng
{
    let network = network.into();
    let rpc = AnchoringRpcConfig {
        host: client.url().into(),
        username: client.username().clone(),
        password: client.password().clone(),
    };
    let mut pub_keys = Vec::new();
    let mut node_cfgs = Vec::new();
    let mut priv_keys = Vec::new();

    for _ in 0..count as usize {
        let (pub_key, priv_key) = btc::gen_btc_keypair_with_rng(network, rng);

        pub_keys.push(pub_key.clone());
        node_cfgs.push(AnchoringNodeConfig::new(rpc.clone()));
        priv_keys.push(priv_key.clone());
    }

    let majority_count = ::majority_count(count);
    let (_, address) = client
        .create_multisig_address(network.into(), majority_count, pub_keys.iter())
        .unwrap();
    let tx = FundingTx::create(client, &address, total_funds).unwrap();

    let genesis_cfg = AnchoringConfig::new(pub_keys, tx);
    for (idx, node_cfg) in node_cfgs.iter_mut().enumerate() {
        node_cfg
            .private_keys
            .insert(address.to_base58check(), priv_keys[idx].clone());
    }

    (genesis_cfg, node_cfgs)
}

/// Same as [`gen_anchoring_testnet_config_with_rng`](fn.gen_anchoring_testnet_config_with_rng.html)
/// but it uses default random number generator.
pub fn gen_anchoring_testnet_config(client: &AnchoringRpc,
                                    network: btc::Network,
                                    count: u8,
                                    total_funds: u64)
                                    -> (AnchoringConfig, Vec<AnchoringNodeConfig>) {
    let mut rng = thread_rng();
    gen_anchoring_testnet_config_with_rng(client, network, count, total_funds, &mut rng)
}
