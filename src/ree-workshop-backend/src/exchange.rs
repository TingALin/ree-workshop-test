pub use crate::log::*;
use crate::{
    errors::ExchangeError,
    log,
    memory::{mutate_state, read_state, set_state, BLOCKS, TX_RECORDS},
    state::{ExchangeState, Game, GameAndGamer, GameStatus, RegisterInfo},
    *,
};
use candid::Principal;
use ic_cdk::{api::management_canister::bitcoin::Satoshi, post_upgrade};
use ic_cdk_macros::{query, update};
use ree_types::{
    bitcoin::{Network, Psbt},
    exchange_interfaces::{
        ExecuteTxArgs, ExecuteTxResponse, GetPoolInfoArgs, GetPoolInfoResponse,
        GetPoolListResponse, NewBlockArgs, NewBlockResponse, PoolBasic, PoolInfo, RollbackTxArgs,
        RollbackTxResponse,
    },
    psbt::ree_pool_sign,
    schnorr::request_ree_pool_address,
    CoinBalance, CoinId, Intention,
};

#[update]
async fn init(
    game_duration: Seconds,
    game_start_time: SecondTimestamp,
    gamer_register_fee: Satoshi,
    claim_cooling_down: Seconds,
    cookie_amount_per_claim: u128,
    max_cookies: u128,
    orchestrator: Principal,
    ii_canister: Principal,
) -> Result<(), String> {
    let rune_id = CoinId::rune(840000, 846);
    let (untweaked, _tweaked, addr) = request_ree_pool_address(
        "key_1",
        vec![rune_id.to_string().as_bytes().to_vec()],
        Network::Testnet4,
    )
    .await?;

    let init_game = Game {
        is_end: false,
        game_duration,
        game_start_time,
        gamer_register_fee,
        claim_cooling_down,
        cookie_amount_per_claim,
        max_cookies,
        claimed_cookies: 0,
        gamer: None,
    };

    let init_state = ExchangeState {
        states: vec![],
        key: untweaked,
        address: addr.to_string(),
        game: init_game,
        orchestrator,
        ii_canister,
        address_principal_map: None,
        game_status: GameStatus::Init,
    };
    set_state(init_state);
    Ok(())
}

#[query]
fn get_exchange_state() -> ExchangeState {
    read_state(|s| s.clone())
}

#[query]
pub fn get_game_and_gamer_infos(gamer: Address) -> GameAndGamer {
    read_state(|es| {
        let game = es.game.clone();
        let gamer_info = es.game.gamer.as_ref().and_then(|g| g.get(&gamer)).cloned();
        GameAndGamer {
            game_duration: game.game_duration,
            game_start_time: game.game_start_time,
            gamer_register_fee: game.gamer_register_fee,
            claim_cooling_down: game.claim_cooling_down,
            cookie_amount_per_claim: game.cookie_amount_per_claim,
            max_cookies: game.max_cookies,
            claimed_cookies: game.claimed_cookies,
            gamer: if let Some(gamer) = gamer_info {
                Some(gamer)
            } else {
                None
            },
        }
    })
}
// 只有前端才能调用
#[update]
async fn end_game() -> Result<(), ExchangeError> {
    read_state(|es| {
        if es.game_status == GameStatus::Ended {
            return Err(ExchangeError::GameEnd);
        }
        let mut new_state = es.clone();
        new_state.game_status = GameStatus::Ended;
        new_state.game.is_end = true;
        set_state(new_state);
        Ok(())
    })
}

#[query]
pub fn get_register_info() -> RegisterInfo {
    read_state(|es| {
        let untweaked_key = es.key.clone();
        let address = es.address.clone();
        let utxo = es.states.last().and_then(|s| s.utxo.clone());
        let register_fee = es.game.gamer_register_fee;
        let nonce = es.states.last().map_or(0, |s| s.nonce);
        RegisterInfo {
            untweaked_key,
            address,
            utxo,
            register_fee,
            nonce,
        }
    })
}

#[update]
pub fn claim() -> Result<u128, ExchangeError> {
    let principal = ic_cdk::caller();
    let address = read_state(|s| s.address_principal_map.clone())
        .and_then(|map| map.get(&principal).cloned())
        .ok_or(ExchangeError::GamerNotFound(principal.to_text().clone()))?;
    mutate_state(|s| s.game.claim(address))
}

#[update(guard = "ensure_orchestrator")]
pub async fn execute_tx(args: ExecuteTxArgs) -> ExecuteTxResponse {
    let ExecuteTxArgs {
        psbt_hex,
        txid,
        intention_set,
        intention_index,
        zero_confirmed_tx_queue_length: _zero_confirmed_tx_queue_length,
    } = args;
    let raw = hex::decode(psbt_hex).map_err(|_| "invalid psbt".to_string())?;
    let mut psbt = Psbt::deserialize(raw.as_slice()).map_err(|_| "invalid psbt".to_string())?;
    let intention = intention_set.intentions[intention_index as usize].clone();
    let initiator = intention_set.initiator_address.clone();
    let Intention {
        exchange_id: _,
        action,
        action_params: _,
        pool_address,
        nonce,
        pool_utxo_spent,
        pool_utxo_received,
        input_coins,
        output_coins,
    } = intention;

    let _ = read_state(|s| {
        return s
            .address
            .eq(&pool_address)
            .then(|| ())
            .ok_or_else(|| "address not match".to_string());
    });

    match action.as_str() {
        "register" => {
            let (new_state, consumed) = read_state(|es| {
                es.validate_register(
                    txid.clone(),
                    nonce,
                    pool_utxo_spent,
                    pool_utxo_received,
                    input_coins,
                    output_coins,
                    initiator.clone(),
                )
            })
            .map_err(|e| e.to_string())?;

            let key_name = CoinId::rune(840000, 846).to_string();
            ree_pool_sign(
                &mut psbt,
                vec![&consumed],
                "key_1",
                vec![key_name.into_bytes()],
            )
            .await
            .map_err(|e| e.to_string())?;

            log!(INFO, "psbt: {:?}", serde_json::to_string_pretty(&psbt));

            mutate_state(|s| {
                s.game.register_new_gamer(initiator.clone());
                s.game_status = GameStatus::Play;
                s.commit(new_state);
            })
        }
        "withdraw" => {
            let (new_state, consumed) = read_state(|es| {
                es.validate_withdraw(
                    txid.clone(),
                    nonce,
                    pool_utxo_spent,
                    pool_utxo_received,
                    input_coins,
                    output_coins,
                    initiator.clone(),
                )
            })
            .map_err(|e| e.to_string())?;
            let key_name = CoinId::rune(840000, 846).to_string();
            ree_pool_sign(
                &mut psbt,
                vec![&consumed],
                "key_1",
                vec![key_name.into_bytes()],
            )
            .await
            .map_err(|e| e.to_string())?;

            mutate_state(|s| {
                s.commit(new_state);
                s.game.withdraw(initiator.clone())
            })
            .map_err(|e| e.to_string())?;
        }
        _ => return Err("Unknown action".to_string()),
    }

    // Record the transaction as unconfirmed and track which pools it affects
    TX_RECORDS.with_borrow_mut(|m| {
        ic_cdk::println!("new unconfirmed txid: {} in pool: {} ", txid, pool_address);
        let mut record = m.get(&(txid.clone(), false)).unwrap_or_default();
        if !record.pools.contains(&pool_address) {
            record.pools.push(pool_address.clone());
        }
        m.insert((txid.clone(), false), record);
    });

    Ok(psbt.serialize_hex())
}

#[query]
pub fn get_pool_list() -> GetPoolListResponse {
    let address = read_state(|s| s.address.clone());
    vec![PoolBasic {
        name: "REE_COOKIE".to_string(),
        address,
    }]
}

#[query]
pub fn get_pool_info(args: GetPoolInfoArgs) -> GetPoolInfoResponse {
    let pool_address = args.pool_address;
    read_state(|es| match es.last_state() {
        Ok(last_state) => {
            if es.address != pool_address {
                return None;
            }
            let rune_id = CoinId::rune(840000, 846);

            Some(PoolInfo {
                key: es.key.clone(),
                key_derivation_path: vec![CoinId::rune(840000, 846).to_string().into_bytes()],
                name: "HOPE•YOU•GET•RICH".to_string(),
                address: es.address.clone(),
                nonce: last_state.nonce,
                coin_reserved: es
                    .states
                    .last()
                    .map(|_s| {
                        vec![CoinBalance {
                            id: CoinId::rune(840000, 846),
                            value: es
                                .states
                                .last()
                                .and_then(|s| {
                                    s.utxo
                                        .as_ref()
                                        .and_then(|utxo| {
                                            utxo.coins.iter().find(|c| c.id == rune_id)
                                        })
                                        .map(|c| c.value)
                                })
                                .unwrap_or(0),
                        }]
                    })
                    .unwrap_or_default(),
                btc_reserved: last_state.btc_balance(),
                utxos: es
                    .states
                    .last()
                    .and_then(|s| s.utxo.clone())
                    .map(|utxo| vec![utxo])
                    .unwrap_or_default(),
                attributes: "".to_string(),
            })
        }
        Err(_e) => {
            return None;
        }
    })
}

#[update(guard = "ensure_orchestrator")]
pub fn rollback_tx(args: RollbackTxArgs) -> RollbackTxResponse {
    let cookie_pool = read_state(|s| s.address.clone());

    TX_RECORDS.with_borrow_mut(|m| {
        let maybe_unconfirmed_record = m.get(&(args.txid.clone(), false));
        let maybe_confirmed_record = m.get(&(args.txid.clone(), true));
        let record = maybe_confirmed_record.or(maybe_unconfirmed_record).unwrap();
        ic_cdk::println!(
            "rollback txid: {} with pools: {:?}",
            args.txid,
            record.pools
        );

        // Roll back each affected pool to its state before this transaction
        record.pools.iter().for_each(|pool_address| {
            if pool_address.eq(&cookie_pool) {
                // Rollback the state of the pool
                mutate_state(|s| s.rollback(args.txid)).unwrap();
            }
        });
        m.remove(&(args.txid.clone(), false));
        m.remove(&(args.txid.clone(), true));
    });

    Ok(())
}

#[update(guard = "ensure_orchestrator")]
pub fn new_block(args: NewBlockArgs) -> NewBlockResponse {
    match crate::reorg::detect_reorg(Network::Testnet4, args.clone()) {
        Ok(_) => {}
        Err(crate::reorg::ReorgError::DuplicateBlock { height, hash }) => {
            ic_cdk::println!(
                "Duplicate block detected at height {} with hash {}",
                height,
                hash
            );
        }
        Err(crate::reorg::ReorgError::Unrecoverable) => {
            return Err("Unrecoverable reorg detected".to_string());
        }
        Err(crate::reorg::ReorgError::BlockNotFoundInState { height }) => {
            return Err(format!("Block not found in state at height {}", height));
        }
        Err(crate::reorg::ReorgError::Recoverable { height, depth }) => {
            crate::reorg::handle_reorg(height, depth);
        }
    }

    let NewBlockArgs {
        block_height,
        block_hash: _,
        block_timestamp: _,
        confirmed_txids,
    } = args.clone();

    // Store the new block information
    BLOCKS.with_borrow_mut(|m| {
        m.insert(block_height, args);
        ic_cdk::println!("new block {} inserted into blocks", block_height,);
    });

    // Mark transactions as confirmed
    for txid in confirmed_txids {
        TX_RECORDS.with_borrow_mut(|m| {
            if let Some(record) = m.get(&(txid.clone(), false)) {
                m.insert((txid.clone(), true), record.clone());
                ic_cdk::println!("confirm txid: {} with pools: {:?}", txid, record.pools);
            }
        });
    }
    // Calculate the height below which blocks are considered fully confirmed (beyond reorg risk)
    let confirmed_height =
        block_height - crate::reorg::get_max_recoverable_reorg_depth(Network::Testnet4) + 1;

    let exchange_pool_address = read_state(|s| s.address.clone());
    // Finalize transactions in confirmed blocks
    BLOCKS.with_borrow(|m| {
        m.iter()
            .take_while(|(height, _)| *height <= confirmed_height)
            .for_each(|(height, block_info)| {
                ic_cdk::println!("finalizing txs in block: {}", height);
                block_info.confirmed_txids.into_iter().for_each(|txid| {
                    TX_RECORDS.with_borrow_mut(|m| {
                        if let Some(record) = m.get(&(txid.clone(), true)) {
                            record.pools.iter().for_each(|pool| {
                                if pool.eq(&exchange_pool_address) {
                                    mutate_state(|s| {
                                        s.finalize(txid.clone()).map_err(|e| e.to_string())
                                    })
                                    .unwrap();
                                }
                            });
                        }
                    })
                })
            })
    });

    // Clean up old block data that's no longer needed
    BLOCKS.with_borrow_mut(|m| {
        let heights_to_remove: Vec<u32> = m
            .iter()
            .take_while(|(height, _)| *height <= confirmed_height)
            .map(|(height, _)| height)
            .collect();
        for height in heights_to_remove {
            ic_cdk::println!("removing block: {}", height);
            m.remove(&height);
        }
    });

    Ok(())
}

#[update(guard = "is_controller")]
pub fn reset_blocks() {
    BLOCKS.with_borrow_mut(|b| {
        b.clear_new();
    });
}

fn is_controller() -> std::result::Result<(), String> {
    ic_cdk::api::is_controller(&ic_cdk::caller())
        .then(|| ())
        .ok_or("Access denied".to_string())
}

fn ensure_orchestrator() -> std::result::Result<(), String> {
    read_state(|s| {
        s.orchestrator
            .eq(&ic_cdk::caller())
            .then(|| ())
            .ok_or("Access denied".to_string())
    })
}

#[post_upgrade]
fn post_upgrade() {
    log!(
        INFO,
        "Finish Upgrade current version: {}",
        env!("CARGO_PKG_VERSION")
    );
}

ic_cdk::export_candid!();
