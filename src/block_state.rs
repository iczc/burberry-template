use alloy::{
    eips::{eip1559::BaseFeeParams, BlockNumberOrTag},
    primitives::U256,
    providers::Provider,
    rpc::types::{BlockOverrides, Header},
};
use eyre::{ContextCompat, WrapErr};
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Default, Clone, Copy)]
pub struct BlockInfo {
    pub number: u64,
    pub timestamp: u64,
    pub gas_used: Option<u64>,
    pub gas_limit: Option<u64>,
    pub base_fee_per_gas: u64,
}

impl BlockInfo {
    /// Returns block info for next block
    pub fn get_next_block(&self) -> BlockInfo {
        let gas_used = self.gas_used.unwrap_or_default();
        let gas_limit = self.gas_limit.unwrap_or_default();
        let base_fee_per_gas = BaseFeeParams::ethereum().next_block_base_fee(
            gas_used,
            gas_limit,
            self.base_fee_per_gas,
        );

        BlockInfo {
            number: self.number + 1,
            timestamp: self.timestamp + 12,
            base_fee_per_gas,
            gas_used: None,
            gas_limit: None,
        }
    }
}

impl From<Header> for BlockInfo {
    fn from(value: Header) -> Self {
        Self {
            number: value.number,
            timestamp: value.timestamp,
            gas_used: Some(value.gas_used),
            gas_limit: Some(value.gas_limit),
            base_fee_per_gas: value.base_fee_per_gas.unwrap_or_default(),
        }
    }
}

impl From<BlockInfo> for BlockOverrides {
    fn from(value: BlockInfo) -> Self {
        Self {
            number: Some(U256::from(value.number)),
            time: Some(value.timestamp),
            base_fee: Some(U256::from(value.base_fee_per_gas)),
            ..Default::default()
        }
    }
}

pub struct BlockState {
    latest_block: BlockInfo,
    next_block: BlockInfo,
}

impl BlockState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn setup(&mut self, provider: Arc<dyn Provider>) -> eyre::Result<()> {
        let latest_block = provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .context("failed to get latest block")?
            .context("latest block not found")?;

        self.update_block_info(latest_block.header.clone());

        info!("latest block synced: {}", latest_block.header.number);
        Ok(())
    }

    /// Return info for the next block
    pub fn get_next_block(&self) -> BlockInfo {
        self.next_block
    }

    /// Return info for the latest block
    pub fn get_latest_block(&self) -> BlockInfo {
        self.latest_block
    }

    pub fn update_block_info<T: Into<BlockInfo>>(&mut self, latest_block: T) {
        self.latest_block = latest_block.into();
        self.next_block = self.latest_block.get_next_block();
    }
}

impl Default for BlockState {
    fn default() -> Self {
        let latest_block = BlockInfo::default();
        Self {
            next_block: latest_block.get_next_block(),
            latest_block,
        }
    }
}
