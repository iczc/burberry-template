use alloy::{
    primitives::B256,
    providers::{Provider, ProviderBuilder, WsConnect},
    signers::{local::PrivateKeySigner, Signer},
};
use burberry::{
    collector::{BlockCollector, MempoolCollector},
    map_collector, map_executor, Engine,
};
use burberry_template::{
    executor::{EchoExecutor, FlashbotsSender},
    strategy::{Config, Strategy},
    types::{Action, Event},
};
use clap::Parser;
use std::sync::Arc;
use tracing::Level;
use tracing_subscriber::{filter, prelude::*};

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(long, env = "ETH_RPC_URL")]
    pub rpc_url: String,

    #[arg(long, env = "PRIVATE_KEY")]
    pub private_key: B256,

    #[arg(long, env = "RELAY_KEY")]
    pub relay_key: B256,

    #[command(flatten)]
    pub config: Config,
}

#[tokio::main]
async fn main() {
    // Set up tracing and parse args.
    let filter = filter::Targets::new()
        .with_target("burberry_template", Level::INFO)
        .with_target("burberry", Level::INFO);
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    let args = Args::parse();

    let ws = WsConnect::new(args.rpc_url);
    let provider = ProviderBuilder::new()
        .connect_ws(ws)
        .await
        .expect("failed to create ws provider");

    let provider: Arc<dyn Provider<_>> = Arc::new(provider);
    let chain_id = provider
        .get_chain_id()
        .await
        .expect("failed to get chain id");

    let searcher_signer = PrivateKeySigner::from_bytes(&args.private_key)
        .expect("failed to parse private key")
        .with_chain_id(Some(chain_id));

    let relay_signer =
        PrivateKeySigner::from_bytes(&args.relay_key).expect("failed to parse relay key");

    let mut engine = Engine::new();

    let mempool_collector = MempoolCollector::new(Arc::clone(&provider));
    let block_collector = BlockCollector::new(Arc::clone(&provider));

    engine.add_collector(map_collector!(mempool_collector, Event::Transaction));
    engine.add_collector(map_collector!(block_collector, Event::Block));

    let strategy = Strategy::new(
        Arc::clone(&provider),
        Arc::new(args.config),
        searcher_signer.address(),
    );
    engine.add_strategy(Box::new(strategy));

    let flashbots_sender = FlashbotsSender::new(provider, vec![searcher_signer], relay_signer);
    engine.add_executor(map_executor!(flashbots_sender, Action::SendToBundle));
    engine.add_executor(map_executor!(
        EchoExecutor::default(),
        Action::EchoTransaction
    ));

    engine.run_and_join().await.unwrap()
}
