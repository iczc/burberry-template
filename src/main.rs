use alloy::providers::ProviderBuilder;
use alloy::providers::{Provider, WsConnect};
use burberry::collector::{BlockCollector, MempoolCollector};
use burberry::{map_collector, map_executor, Engine};
use burberry_template::{
    executor::EchoExecutor,
    strategy::EchoStrategy,
    types::{Action, Event},
};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let ws = WsConnect::new("wss://eth.merkle.io");
    let provider = ProviderBuilder::new()
        .connect_ws(ws)
        .await
        .expect("fail to create ws provider");

    let provider: Arc<dyn Provider<_>> = Arc::new(provider);

    let mut engine = Engine::new();

    let mempool_collector = MempoolCollector::new(Arc::clone(&provider));
    let block_collector = BlockCollector::new(Arc::clone(&provider));

    engine.add_collector(map_collector!(mempool_collector, Event::Transaction));
    engine.add_collector(map_collector!(block_collector, Event::Block));

    engine.add_strategy(Box::new(EchoStrategy));

    engine.add_executor(map_executor!(EchoExecutor::default(), Action::EchoBlock));
    engine.add_executor(map_executor!(
        EchoExecutor::default(),
        Action::EchoTransaction
    ));

    engine.run_and_join().await.unwrap()
}
