use alloy::primitives::B256;
use alloy::rpc::types::{Header, Transaction};

/// Core Event enum for the current strategy.
#[derive(Debug, Clone)]
pub enum Event {
    Block(Header),
    Transaction(Transaction),
}

/// Core Action enum for the current strategy.
#[derive(Debug, Clone)]
pub enum Action {
    EchoBlock(u64),
    EchoTransaction(B256),
}
