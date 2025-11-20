use crate::flashbots::FlashbotsBroadcaster;
use alloy::{
    network::{eip2718::Encodable2718, EthereumWallet, TransactionBuilder},
    primitives::{keccak256, Address, Bytes},
    providers::Provider,
    rpc::types::eth::TransactionRequest,
    rpc::types::mev::EthSendBundle,
    signers::local::PrivateKeySigner,
};
use burberry::Executor;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

#[derive(Default)]
pub struct EchoExecutor<T>(PhantomData<T>);

#[async_trait::async_trait]
impl<T: Debug + Send + Sync> Executor<T> for EchoExecutor<T> {
    async fn execute(&self, action: T) -> eyre::Result<()> {
        println!("action: {:?}", action);
        Ok(())
    }
}

pub struct FlashbotsSender {
    provider: Arc<dyn Provider>,
    signers: HashMap<Address, EthereumWallet>,
    broadcaster: FlashbotsBroadcaster,
}

#[derive(Clone, Debug)]
pub struct BundleRequest {
    pub tx: TransactionRequest,
    pub block: u64,
}

impl FlashbotsSender {
    pub fn new(
        provider: Arc<dyn Provider>,
        signers: Vec<PrivateKeySigner>,
        relay_signer: PrivateKeySigner,
    ) -> Self {
        let signers: HashMap<_, _> = signers
            .into_iter()
            .map(|s| (s.address(), EthereumWallet::new(s)))
            .collect();

        for signer in signers.keys() {
            tracing::info!("setting up signer {:#x}", signer);
        }

        let broadcaster = FlashbotsBroadcaster::new(Some(relay_signer))
            .unwrap()
            .with_default_relays()
            .unwrap();

        Self {
            provider,
            signers,
            broadcaster,
        }
    }
}

#[async_trait::async_trait]
impl Executor<BundleRequest> for FlashbotsSender {
    fn name(&self) -> &str {
        "FlashbotsSender"
    }

    async fn execute(&self, action: BundleRequest) -> eyre::Result<()> {
        let mut tx = action.tx;

        let account = match tx.from {
            Some(v) => v,
            None => {
                tracing::error!("missing sender address");
                return Ok(());
            }
        };

        let signer = match self.signers.get(&account) {
            Some(v) => v,
            None => {
                tracing::error!("missing signer for {:#x}", account);
                return Ok(());
            }
        };

        if tx.nonce.is_none() {
            let nonce = match self.provider.get_transaction_count(account).await {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(?account, "failed to get nonce: {err:#}");
                    return Ok(());
                }
            };

            tx.set_nonce(nonce);
        }

        let raw_tx: Bytes = match tx.build(signer).await {
            Ok(v) => v.encoded_2718().into(),
            Err(err) => {
                tracing::error!(?account, "failed to build tx: {err:#}");
                return Ok(());
            }
        };

        tracing::debug!(?account, tx = ?raw_tx, "signed tx");

        let tx_hash = keccak256(&raw_tx);
        let bundle = EthSendBundle {
            txs: vec![raw_tx],
            block_number: action.block,
            ..Default::default()
        };

        let send_result = self.broadcaster.broadcast_bundle(bundle).await;
        match send_result {
            Ok(_) => {
                tracing::info!(?account, tx = ?tx_hash, "sent tx");
            }
            Err(err) => {
                tracing::error!(?account, tx = ?tx_hash, "failed to send tx: {err:#}");
                return Ok(());
            }
        }

        Ok(())
    }
}
