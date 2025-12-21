use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::Provider,
    rpc::types::TransactionRequest,
};
use burberry::{submit_action, ActionSubmitter};
use clap::Parser;
use eyre::eyre;
use std::sync::Arc;
use tracing::{error, info};

use super::types::{Action, Event};
use crate::{
    block_state::{BlockInfo, BlockState},
    executor::BundleRequest,
};

pub struct Strategy {
    provider: Arc<dyn Provider>,
    block_state: BlockState,
    config: Arc<Config>,
    sender: Address,
}

#[derive(Debug, Clone, Parser)]
pub struct Config {
    #[arg(long, help = "Target contract address")]
    contract_address: Address,
    #[arg(long, help = "Transaction calldata (hex encoded)")]
    calldata: Bytes,
    #[arg(
        long,
        default_value = "1000000000",
        help = "Max priority fee per gas in wei"
    )]
    max_priority_fee: u128,
}

impl Strategy {
    pub fn new(provider: Arc<dyn Provider>, config: Arc<Config>, sender: Address) -> Self {
        Self {
            provider,
            config,
            block_state: BlockState::new(),
            sender,
        }
    }

    async fn build_tx(&self, next_block: BlockInfo) -> eyre::Result<TransactionRequest> {
        let nonce = self
            .provider
            .get_transaction_count(self.sender)
            .await
            .map_err(|e| eyre!("Failed to get nonce {:?}", e))?;

        let max_fee_per_gas = next_block.base_fee_per_gas as u128 + self.config.max_priority_fee;

        let gas_used = 1000000_u64;

        let tx: TransactionRequest = TransactionRequest::default()
            .with_nonce(nonce)
            .with_input(self.config.calldata.clone())
            .with_from(self.sender)
            .with_to(self.config.contract_address)
            .with_max_fee_per_gas(max_fee_per_gas)
            .with_max_priority_fee_per_gas(max_fee_per_gas)
            .with_gas_limit(gas_used * 10 / 7);

        Ok(tx)
    }
}
#[async_trait::async_trait]
impl burberry::Strategy<Event, Action> for Strategy {
    async fn sync_state(
        &mut self,
        _submitter: Arc<dyn ActionSubmitter<Action>>,
    ) -> eyre::Result<()> {
        self.block_state.setup(self.provider.clone()).await.unwrap();

        Ok(())
    }

    async fn process_event(&mut self, event: Event, submitter: Arc<dyn ActionSubmitter<Action>>) {
        match event {
            Event::Block(block) => {
                info!("found new block: {}", block.number);
                self.block_state.update_block_info(block.clone());
                let next_block = self.block_state.get_next_block();

                match self.build_tx(next_block).await {
                    Ok(tx) => {
                        let bundle_request = BundleRequest {
                            tx,
                            block: next_block.number,
                        };
                        submit_action!(submitter, Action::SendToBundle, bundle_request);
                    }
                    Err(e) => {
                        error!("Failed to build transaction: {}", e);
                    }
                }
            }
            Event::Transaction(tx) => {
                submit_action!(submitter, Action::EchoTransaction, *tx.inner.tx_hash());
            }
        }
    }
}
