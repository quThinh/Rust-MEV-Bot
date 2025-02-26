use bounded_vec_deque::BoundedVecDeque;
use ethers::signers::{LocalWallet, Signer};
use ethers::solc::info;
use ethers::{
    providers::{Middleware, Provider, Ws, Http},
    types::{BlockNumber, H160, H256, U256, U64},
};
use log::{info, warn};
use std::{collections::HashMap, str::FromStr, sync::Arc};
use tokio::sync::broadcast::Sender;

// we'll update this part later, for now just import the necessary components
use crate::common::constants::{Env, WETH};
use crate::common::streams::{Event, NewBlock};
use crate::common::pools::{load_all_pools, Pool};
use crate::common::tokens::load_all_tokens;
use crate::common::utils::{calculate_next_block_base_fee, to_h160};
use crate::sandwich::simulation::{extract_swap_info, debug_trace_call, extract_logs, PendingTxInfo, SwapDirection, SwapInfo};

pub async fn run_sandwich_strategy(provider: Arc<Provider<Http>>, event_sender: Sender<Event>) {
    let env = Env::new();

    // load_all_pools:
    // this will load all Uniswap V2 pools that was deployed after the block #10000000
    let (pools, prev_pool_id) = load_all_pools(env.wss_url.clone(), 10000000, 50000)
        .await
        .unwrap();

    // load_all_tokens:
    // this will get all the token information including: name, symbol, symbol, totalSupply
    let block_number = provider.get_block_number().await.unwrap();
    let tokens_map = load_all_tokens(&provider, block_number, &pools, prev_pool_id)
        .await
        .unwrap();
    info!("Tokens map count: {:?}", tokens_map.len());

    // filter pools that don't have both token0 / token1 info
    let pools_vec: Vec<Pool> = pools
        .into_iter()
        .filter(|p| {
            let token0_exists = tokens_map.contains_key(&p.token0);
            let token1_exists = tokens_map.contains_key(&p.token1);
            token0_exists && token1_exists
        })
        .collect();
    info!("Filtered pools by tokens count: {:?}", pools_vec.len());

    let pools_map: HashMap<H160, Pool> = pools_vec
        .clone()
        .into_iter()
        .map(|p| (p.address, p))
        .collect();

    let block = provider
        .get_block(BlockNumber::Latest)
        .await
        .unwrap()
        .unwrap();
    let mut new_block = NewBlock {
        block_number: block.number.unwrap(),
        base_fee: block.base_fee_per_gas.unwrap(),
        next_base_fee: calculate_next_block_base_fee(
            block.gas_used,
            block.gas_limit,
            block.base_fee_per_gas.unwrap(),
        ),
    };

    let mut event_receiver = event_sender.subscribe();

    loop {
        match event_receiver.recv().await {
            Ok(event) => match event {
                Event::Block(block) => {
                    new_block = block;
                    info!("[Block #{:?}]", new_block.block_number);
                }
                Event::PendingTx(mut pending_tx) => {
                    let swap_info =
                        extract_swap_info(&provider, &new_block, &pending_tx, &pools_map).await;
                    info!("{:?}", swap_info);
                }
            },
            _ => {}
        }
    }
}