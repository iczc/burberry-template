use alloy::{
    network::{eip2718::Encodable2718, EthereumWallet, TransactionBuilder},
    primitives::{keccak256, Address, Bytes},
    providers::Provider,
    rpc::types::{
        eth::TransactionRequest,
        mev::{EthBundleHash, EthSendBundle},
    },
    signers::local::PrivateKeySigner,
    transports::TransportResult,
};
use alloy_mev::{BroadcastableCall, Endpoints, EndpointsBuilder};
use burberry::Executor;
use std::{collections::HashMap, fmt::Debug, marker::PhantomData, sync::Arc};

#[derive(Default)]
pub struct EchoExecutor<T>(PhantomData<T>);

#[async_trait::async_trait]
impl<T: Debug + Send + Sync> Executor<T> for EchoExecutor<T> {
    async fn execute(&self, action: T) -> eyre::Result<()> {
        tracing::debug!("action: {:?}", action);
        Ok(())
    }
}

pub struct FlashbotsSender {
    endpoints: Endpoints,
    provider: Arc<dyn Provider>,
    signers: HashMap<Address, EthereumWallet>,
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
        let endpoints = EndpointsBuilder::default()
            .beaverbuild()
            .titan(relay_signer.clone())
            .rsync()
            .flashbots(relay_signer.clone())
            .build();

        let signers: HashMap<_, _> = signers
            .into_iter()
            .map(|s| (s.address(), EthereumWallet::new(s)))
            .collect();

        for signer in signers.keys() {
            tracing::info!("setting up signer {:#x}", signer);
        }

        Self {
            endpoints,
            provider,
            signers,
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

        let responses: Vec<TransportResult<EthBundleHash>> = BroadcastableCall::new(
            &self.endpoints,
            self.provider
                .client()
                .make_request("eth_sendBundle", (bundle,)),
        )
        .await;

        tracing::info!(?account, tx = ?tx_hash, "sent tx");

        for (response, endpoint) in responses.iter().zip(self.endpoints.iter()) {
            match response {
                Ok(_) => {
                    tracing::debug!("bundle sent to {}", endpoint.url)
                }
                Err(err) => {
                    tracing::error!("failed to send bundle to {}: {err:#}", endpoint.url);
                }
            }
        }

        Ok(())
    }
}
