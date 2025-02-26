use anyhow::Result;
use ethers::providers::{Provider, Ws, Http};
use ethers::solc::info;
use ethers_providers::Middleware;
use log::info;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::broadcast::{self, Sender};
use tokio::task::JoinSet;

use sandooo::common::constants::Env;
use sandooo::common::streams::{stream_new_blocks, stream_pending_transactions, Event};
use sandooo::common::utils::setup_logger;
use sandooo::sandwich::strategy::run_sandwich_strategy;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    setup_logger().unwrap();
    
    info!("Starting Sandooo");

    let env = Env::new();

    info!("Connected to {}", env.wss_url);
    let provider = Provider::<Http>::try_from(
             "https://docs-demo.quiknode.pro"
         ).expect("could not instantiate HTTP Provider");

    let ws = Ws::connect("wss://mainnet.infura.io/ws/v3/a8432db7afb8493bb38b3bb51b060869")
        .await
        .unwrap();
    let ws_arc = Arc::new(Provider::new(ws));
    let provider_arc = Arc::new(provider);

    // Example usage of Middleware trait function
    // Check if the node supports the debug_traceCall method
    // let methods: Vec<String> = provider_arc.request("rpc_modules", ()).await.unwrap_or_default();

    let (event_sender, _): (Sender<Event>, _) = broadcast::channel(512);
    let mut set = JoinSet::new();
    // spawn a new async task and add it to JoinSet
    set.spawn(stream_new_blocks(ws_arc.clone(), event_sender.clone()));
    set.spawn(stream_pending_transactions(
        ws_arc.clone(),
        event_sender.clone(),
    ));
    set.spawn(run_sandwich_strategy(
        provider_arc.clone(),
        event_sender.clone(),
    ));

    while let Some(res) = set.join_next().await {
        info!("{:?}", res);
    }

    // Ok("REASON")
}
