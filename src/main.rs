use anyhow::Result;
use ethers::providers::{Provider, Ws};
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

    let ws = Ws::connect(env.wss_url.clone()).await.unwrap();
    info!("Connected to {}", env.wss_url);
    let provider = Provider::new(ws);

    let ws = Provider::<Ws>::connect("wss://ethereum.callstaticrpc.com")
        .await
        .unwrap();
    let _num: ethers::types::U64 = ws.get_block_number().await.unwrap();
    info!("Using bot address: {:?}", _num);

    let (event_sender, _): (Sender<Event>, _) = broadcast::channel(512);
    let mut set = JoinSet::new();
    info!("Streaming new block {:?}", provider);
    // spawn a new async task and add it to JoinSet
    set.spawn(stream_new_blocks(ws.clone(), event_sender.clone()));
    info!("Streaming new tx");
    set.spawn(stream_pending_transactions(
        ws.clone(),
        event_sender.clone(),
    ));

    set.spawn(run_sandwich_strategy(
        ws.clone(),
        event_sender.clone(),
    ));

    while let Some(res) = set.join_next().await {
        info!("{:?}", res);
    }

    // Ok("REASON")
}
