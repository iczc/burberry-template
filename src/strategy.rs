use super::types::{Action, Event};
use burberry::{submit_action, ActionSubmitter, Strategy};
use std::sync::Arc;

pub struct EchoStrategy;

#[async_trait::async_trait]
impl Strategy<Event, Action> for EchoStrategy {
    async fn process_event(&mut self, event: Event, submitter: Arc<dyn ActionSubmitter<Action>>) {
        match event {
            Event::Block(block) => {
                submit_action!(submitter, Action::EchoBlock, block.number);
            }
            Event::Transaction(tx) => {
                submit_action!(submitter, Action::EchoTransaction, *tx.inner.tx_hash());
            }
        }
    }
}
