use alloy::{
    primitives::B256,
    rpc::types::{Header, Transaction},
};

use crate::executor::BundleRequest;

/// Core Event enum for the current strategy.
#[derive(Debug, Clone)]
pub enum Event {
    Block(Header),
    Transaction(Transaction),
}

/// Core Action enum for the current strategy.
#[derive(Debug, Clone)]
pub enum Action {
    SendToBundle(BundleRequest),
    EchoTransaction(B256),
}
