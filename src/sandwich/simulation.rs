use anyhow::Result;
use eth_encode_packed::ethabi::ethereum_types::{H160 as eH160, U256 as eU256};
use eth_encode_packed::{SolidityDataType, TakeLastXBytes};
use ethers::abi::ParamType;
use ethers::prelude::*;
use ethers::providers::{Provider, Ws};
use ethers::types::{transaction::eip2930::AccessList, Bytes, H160, H256, I256, U256, U64};
use log::info;
use revm::primitives::{Bytecode, U256 as rU256};
use std::{collections::HashMap, default::Default, str::FromStr, sync::Arc};
use crate::common::pools::Pool;
use crate::common::constants::{WETH, WETH_BALANCE_SLOT};
use crate::common::streams::{NewBlock, NewPendingTx};
use crate::common::utils::{create_new_wallet, is_weth, to_h160};

#[derive(Debug, Clone, Default)]
pub struct PendingTxInfo {
    pub pending_tx: NewPendingTx,
    pub touched_pairs: Vec<SwapInfo>,
}

#[derive(Debug, Clone)]
pub enum SwapDirection {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct SwapInfo {
    pub tx_hash: H256,
    pub target_pair: H160,
    pub main_currency: H160,
    pub target_token: H160,
    pub version: u8,
    pub token0_is_main: bool,
    pub direction: SwapDirection,
}

pub static V2_SWAP_EVENT_ID: &str = "0xd78ad95f";

pub async fn debug_trace_call(
    provider: &Provider<Http>,
    new_block: &NewBlock,
    pending_tx: &NewPendingTx,
) -> Result<Option<CallFrame>> {
    let mut opts = GethDebugTracingCallOptions::default();
    let mut call_config = CallConfig::default();
    call_config.with_log = Some(true); // 👈 make sure we are getting logs

    opts.tracing_options.tracer = Some(GethDebugTracerType::BuiltInTracer(
        GethDebugBuiltInTracerType::CallTracer,
    ));
    opts.tracing_options.tracer_config = Some(GethDebugTracerConfig::BuiltInTracer(
        GethDebugBuiltInTracerConfig::CallTracer(call_config),
    ));

    let block_number = new_block.block_number;
    let mut tx = pending_tx.tx.clone();
    let nonce = provider
        .get_transaction_count(tx.from, Some(block_number.into()))
        .await
        .unwrap_or_default();
    tx.nonce = nonce;
    // let trace = provider
    //     .debug_trace_call(&tx, Some(block_number.into()), opts)
    //     .await;
    let trace = provider
        .debug_trace_call(&tx, Some(block_number.into()), opts)
        .await;

    match trace {
        Ok(trace) => match trace {
            GethTrace::Known(call_tracer) => match call_tracer {
                GethTraceFrame::CallTracer(frame) => Ok(Some(frame)),
                _ => Ok(None),
            },
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

pub fn extract_logs(call_frame: &CallFrame, logs: &mut Vec<CallLogFrame>) {
    if let Some(ref logs_vec) = call_frame.logs {
        logs.extend(logs_vec.iter().cloned());
    }

    if let Some(ref calls_vec) = call_frame.calls {
        for call in calls_vec {
            extract_logs(call, logs);
        }
    }
}

pub async fn extract_swap_info(
    provider: &Arc<Provider<Http>>,
    new_block: &NewBlock,
    pending_tx: &NewPendingTx,
    pools_map: &HashMap<H160, Pool>,
) -> Result<Vec<SwapInfo>> {
    let tx_hash = pending_tx.tx.hash;
    let mut swap_info_vec = Vec::new();

    let frame = debug_trace_call(provider, new_block, pending_tx).await?;
    if frame.is_none() {
        return Ok(swap_info_vec);
    }
    let frame = frame.unwrap();

    let mut logs = Vec::new();
    extract_logs(&frame, &mut logs);

    for log in &logs {
        match &log.topics {
            Some(topics) => {
                if topics.len() > 1 {
                    let selector = &format!("{:?}", topics[0])[0..10];
                    let is_v2_swap = selector == V2_SWAP_EVENT_ID;
                    if is_v2_swap {
                        let pair_address = log.address.unwrap();

                        // filter out the pools we have in memory only
                        let pool = pools_map.get(&pair_address);
                        if pool.is_none() {
                            continue;
                        }
                        let pool = pool.unwrap();

                        let token0 = pool.token0;
                        let token1 = pool.token1;

                        let token0_is_weth = is_weth(token0);
                        let token1_is_weth = is_weth(token1);

                        // filter WETH pairs only
                        if !token0_is_weth && !token1_is_weth {
                            continue;
                        }

                        let (main_currency, target_token, token0_is_main) = if token0_is_weth {
                            (token0, token1, true)
                        } else {
                            (token1, token0, false)
                        };

                        let (in0, _, _, out1) = match ethers::abi::decode(
                            &[
                                ParamType::Uint(256),
                                ParamType::Uint(256),
                                ParamType::Uint(256),
                                ParamType::Uint(256),
                            ],
                            log.data.as_ref().unwrap(),
                        ) {
                            Ok(input) => {
                                let uints: Vec<U256> = input
                                    .into_iter()
                                    .map(|i| i.to_owned().into_uint().unwrap())
                                    .collect();
                                (uints[0], uints[1], uints[2], uints[3])
                            }
                            _ => {
                                let zero = U256::zero();
                                (zero, zero, zero, zero)
                            }
                        };

                        let zero_for_one = (in0 > U256::zero()) && (out1 > U256::zero());

                        let direction = if token0_is_main {
                            if zero_for_one {
                                SwapDirection::Buy
                            } else {
                                SwapDirection::Sell
                            }
                        } else {
                            if zero_for_one {
                                SwapDirection::Sell
                            } else {
                                SwapDirection::Buy
                            }
                        };

                        let swap_info = SwapInfo {
                            tx_hash,
                            target_pair: pair_address,
                            main_currency,
                            target_token,
                            version: 2,
                            token0_is_main,
                            direction,
                        };
                        swap_info_vec.push(swap_info);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(swap_info_vec)
}