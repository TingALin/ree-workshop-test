use crate::memory::mutate_state;
use crate::*;
use candid::Deserialize;
pub use errors::*;
use ic_cdk::api::management_canister::bitcoin::Satoshi;
use ic_stable_structures::storable::Bound;
use ic_stable_structures::Storable;
pub use log::*;
use ree_types::{CoinBalance, CoinBalances, CoinId, InputCoin, OutputCoin};
use std::borrow::Cow;
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize, Clone, CandidType)]
pub struct ExchangeState {
    pub states: Vec<PoolState>,
    pub key: Pubkey,
    pub address: String,
    pub ii_canister: Principal,
    pub orchestrator: Principal,
    pub address_principal_map: Option<BTreeMap<Principal, Address>>,
    pub game: Game,
    pub game_status: GameStatus,
}

impl Storable for ExchangeState {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(bincode::serialize(self).unwrap())
    }
    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        bincode::deserialize(bytes.as_ref()).unwrap()
    }
    const BOUND: Bound = Bound::Unbounded;
}

impl ExchangeState {
    pub fn validate_register(
        &self,
        txid: Txid,
        nonce: u64,
        pool_utxo_spend: Vec<String>,
        pool_utxo_receive: Vec<Utxo>,
        input_coins: Vec<InputCoin>,
        output_coins: Vec<OutputCoin>,
        address: Address,
    ) -> Result<(PoolState, Utxo)> {
        if self.game_status == GameStatus::Ended {
            return Err(ExchangeError::GameEnd);
        }
        if self.game.gamer.as_ref().unwrap().contains_key(&address) {
            return Err(ExchangeError::GamerAlreadyExist(address.clone()));
        }

        (input_coins.len() == 1
            && output_coins.is_empty()
            && input_coins[0].coin.id.eq(&CoinId::btc())
            && input_coins[0].coin.value > self.game.gamer_register_fee as u128)
            .then(|| {})
            .ok_or(ExchangeError::InvalidSignPsbtArgs(format!(
                "input_coins: {:?}, output_coins: {:?}",
                input_coins, output_coins
            )))?;
        let last_state = self.last_state()?;
        (last_state.nonce == nonce)
            .then(|| ())
            .ok_or(ExchangeError::PoolStateExpired(last_state.nonce))?;

        last_state
            .utxo
            .as_ref()
            .ok_or(ExchangeError::LastStateNotFound)?
            .outpoint()
            .eq(pool_utxo_spend
                .last()
                .ok_or(ExchangeError::InvalidSignPsbtArgs(
                    "pool_utxo_spend is empty".to_string(),
                ))?)
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(format!(
                "pool_utxo_spend: {:?}, last_state_utxo: {:?}",
                pool_utxo_spend,
                last_state.clone().utxo
            )))?;

        // the pool_utxo_receive should exist
        let pool_new_outpoint = pool_utxo_receive.last().map(|s| s.clone()).ok_or(
            ExchangeError::InvalidSignPsbtArgs("pool_utxo_receive not found".to_string()),
        )?;
        let pool_new_utxo = Utxo::try_from(
            pool_new_outpoint.outpoint(),
            last_state
                .clone()
                .utxo
                .ok_or(ExchangeError::LastStateNotFound)?
                .coins,
            last_state
                .clone()
                .utxo
                .ok_or(ExchangeError::LastStateNotFound)?
                .sats
                .checked_add(self.game.gamer_register_fee)
                .ok_or(ExchangeError::Overflow)?,
        )
        .map_err(|e| ExchangeError::InvalidSignPsbtArgs(e.to_string()))?;

        let new_state = PoolState {
            id: Some(txid),
            nonce: last_state
                .nonce
                .checked_add(1)
                .ok_or(ExchangeError::Overflow)?,
            utxo: Some(pool_new_utxo),
            user_action: UserAction::Register(address.clone()),
        };

        Ok((
            new_state,
            last_state
                .clone()
                .utxo
                .ok_or(ExchangeError::LastStateNotFound)?,
        ))
    }

    pub fn last_state(&self) -> Result<PoolState> {
        self.states
            .last()
            .cloned()
            .ok_or(ExchangeError::LastStateNotFound)
            .inspect_err(|e| log!(ERROR, "{}", e))
    }

    pub(crate) fn commit(&mut self, state: PoolState) {
        self.states.push(state);
    }

    pub fn validate_withdraw(
        &self,
        txid: Txid,
        nonce: u64,
        pool_utxo_spend: Vec<String>,
        pool_utxo_receive: Vec<Utxo>,
        input_coins: Vec<InputCoin>,
        output_coins: Vec<OutputCoin>,
        initiator_address: Address,
    ) -> Result<(PoolState, Utxo)> {
        if self.game_status == GameStatus::Ended {
            return Err(ExchangeError::GameEnd);
        }
        let gamer = self
            .game
            .gamer
            .as_ref()
            .and_then(|g| g.get(&initiator_address))
            .ok_or(ExchangeError::GamerNotFound(initiator_address.clone()))?;

        (input_coins.len() == 0 && output_coins.len() == 2)
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(format!(
                "input_coins: {:?}, output_coins: {:?}",
                input_coins, output_coins
            )))?;
        let pool_expected_spend_rune = CoinBalance {
            id: CoinId::rune(840000, 846),
            value: gamer.cookies,
        };
        assert!(
            output_coins.len() == 1
                && input_coins.is_empty()
                && output_coins[0].coin.id.eq(&pool_expected_spend_rune.id)
                && output_coins[0].coin.value == pool_expected_spend_rune.value
                && output_coins[0].to.eq(&initiator_address),
        );
        let last_state = self.last_state()?;
        (last_state.nonce == nonce)
            .then(|| ())
            .ok_or(ExchangeError::PoolStateExpired(last_state.nonce))?;

        (pool_utxo_spend.len() == 1
            && pool_utxo_spend.contains(
                &last_state
                    .clone()
                    .utxo
                    .ok_or(ExchangeError::LastStateNotFound)?
                    .outpoint(),
            ))
        .then(|| ())
        .ok_or(ExchangeError::InvalidSignPsbtArgs(format!(
            "Pool Utxo Spend not eq last pool state utxos, pool_utxo_spend: {:?}, last_state: {:?}",
            pool_utxo_spend, last_state
        )))?;
        let pool_new_outpoint = pool_utxo_receive.first().map(|s| s.clone()).ok_or(
            ExchangeError::InvalidSignPsbtArgs("pool_utxo_receive not found".to_string()),
        )?;

        let last_rune_info = last_state
            .utxo
            .as_ref()
            .and_then(|utxo: &Utxo| {
                utxo.coins
                    .iter()
                    .find(|c| c.id == CoinId::rune(840000, 846))
            })
            .ok_or(ExchangeError::LastStateNotFound)?;

        let coins = CoinBalances::new();
        coins.clone().add_coin(&CoinBalance {
            id: CoinId::rune(840000, 846),
            value: last_rune_info
                .value
                .checked_sub(pool_expected_spend_rune.value as u128)
                .ok_or(ExchangeError::Overflow)?,
        });
        let new_utxo = Utxo::try_from(
            pool_new_outpoint.outpoint(),
            coins,
            last_state
                .utxo
                .as_ref()
                .ok_or(ExchangeError::LastStateNotFound)?
                .sats,
        )
        .map_err(|e| ExchangeError::InvalidSignPsbtArgs(e.to_string()))?;

        let new_state = PoolState {
            id: Some(txid),
            nonce: last_state
                .nonce
                .checked_add(1)
                .ok_or(ExchangeError::Overflow)?,
            utxo: Some(new_utxo),
            user_action: UserAction::Withdraw(initiator_address.clone()),
        };

        return Ok((
            new_state,
            last_state.utxo.ok_or(ExchangeError::LastStateNotFound)?,
        ));
    }

    pub(crate) fn finalize(&mut self, txid: Txid) -> Result<()> {
        let idx = self
            .states
            .iter()
            .position(|s| s.id == Some(txid))
            .ok_or(ExchangeError::InvalidState("txid not found".to_string()))?;

        if idx == 0 {
            return Ok(());
        }

        self.states.rotate_left(idx);
        self.states.truncate(self.states.len() - idx);

        Ok(())
    }

    pub(crate) fn rollback(&mut self, txid: Txid) -> Result<()> {
        let idx = self
            .states
            .iter()
            .position(|state| state.id == Some(txid))
            .ok_or(ExchangeError::InvalidState("txid not found".to_string()))?;
        if idx == 0 {
            return Err(ExchangeError::InvalidState(
                "Should not rollback index 0 state".to_string(),
            ));
        }

        while self.states.len() > idx {
            let state = self.states.pop().ok_or(ExchangeError::InvalidState(
                "Should not rollback index 0 state".to_string(),
            ))?;
            match state.user_action {
                UserAction::Init => {
                    // impossible to rollback init state
                    return Err(ExchangeError::InvalidState(
                        "Should not rollback init action".to_string(),
                    ));
                }
                UserAction::Register(address) => {
                    mutate_state(|es| {
                        es.game
                            .gamer
                            .as_mut()
                            .and_then(|g| g.remove(&address))
                            .ok_or(ExchangeError::GamerNotFound(address))
                    })?;
                }
                UserAction::Withdraw(address) => {
                    mutate_state(|es| {
                        es.game
                            .gamer
                            .as_mut()
                            .and_then(|g| g.get_mut(&address))
                            .map(|gamer| {
                                gamer.is_withdrawn = false;
                            })
                            .ok_or(ExchangeError::GamerNotFound(address))
                    })?;
                }
            }
        }
        Ok(())
    }

    pub fn is_ended(&self) -> bool {
        self.game_status == GameStatus::Ended
    }
}

#[derive(Deserialize, Serialize, Clone, CandidType, Debug)]
pub struct PoolState {
    pub id: Option<Txid>,
    pub nonce: u64,
    pub utxo: Option<Utxo>,
    pub user_action: UserAction,
}

impl PoolState {
    pub fn btc_balance(&self) -> u64 {
        self.utxo.as_ref().map_or(0, |utxo| utxo.sats)
    }
}

#[derive(Deserialize, Serialize, Clone, CandidType, Debug)]
pub enum UserAction {
    Init,
    Register(Address),
    Withdraw(Address),
}

#[derive(Deserialize, Serialize, Clone, CandidType, PartialEq, Eq)]
pub enum GameStatus {
    Init,
    Play,
    Ended,
}

#[derive(Deserialize, Serialize, Clone, CandidType)]
pub struct Game {
    pub game_duration: Seconds,
    pub game_start_time: SecondTimestamp,
    pub gamer_register_fee: Satoshi,
    pub claim_cooling_down: Seconds,
    pub cookie_amount_per_claim: u128,
    pub max_cookies: u128,
    pub claimed_cookies: u128,
    pub gamer: Option<BTreeMap<Address, Gamer>>,
    pub is_end: bool,
}

impl Game {
    pub fn register_new_gamer(&mut self, gamer_id: Address) {
        self.gamer
            .get_or_insert_with(BTreeMap::new)
            .insert(gamer_id.clone(), Gamer::new(gamer_id));
    }

    pub fn claim(&mut self, gamer_id: Address) -> Result<u128> {
        self.able_claim(gamer_id.clone())?;
        let gamer = self
            .gamer
            .as_mut()
            .and_then(|g| g.get_mut(&gamer_id))
            .ok_or(ExchangeError::GamerNotFound(gamer_id.clone()))?;
        // Check for overflow first
        self.claimed_cookies = self
            .claimed_cookies
            .checked_add(self.cookie_amount_per_claim)
            .ok_or(ExchangeError::Overflow)?;
        gamer.claim(self.cookie_amount_per_claim)?;
        let new_cookies_balance = gamer.cookies;
        // No need to re-insert the gamer since we modified it through the mutable reference
        Ok(new_cookies_balance)
    }

    pub fn able_claim(&mut self, gamer_id: Address) -> Result<()> {
        let remaining_cookies = self.max_cookies - self.claimed_cookies;
        if remaining_cookies < self.cookie_amount_per_claim {
            return Err(ExchangeError::CookieBalanceInsufficient(remaining_cookies));
        }
        if let Some(gamer) = self.gamer.as_mut().and_then(|g| g.get_mut(&gamer_id)) {
            if gamer.is_withdrawn {
                return Err(ExchangeError::GamerWithdrawRepeatedly(gamer_id));
            }
            if gamer.last_click_time + self.claim_cooling_down
                > (ic_cdk::api::time() / 1000_000_000)
            {
                return Err(ExchangeError::GamerCoolingDown(
                    gamer_id,
                    gamer.last_click_time + self.claim_cooling_down,
                ));
            }
            Ok(())
        } else {
            Err(ExchangeError::GamerNotFound(gamer_id))
        }
    }

    pub fn withdraw(&mut self, gamer_id: Address) -> Result<u128> {
        let gamer = self
            .gamer
            .as_mut()
            .and_then(|g| g.get_mut(&gamer_id))
            .ok_or(ExchangeError::GamerNotFound(gamer_id.clone()))?;

        if self.is_end {
            if !gamer.is_withdrawn {
                gamer.is_withdrawn = true;
                Ok(gamer.cookies)
            } else {
                Err(ExchangeError::GamerWithdrawRepeatedly(gamer_id))
            }
        } else {
            Err(ExchangeError::GameNotEnd)
        }
    }
}

#[derive(Deserialize, Serialize, Clone, CandidType)]
pub struct Gamer {
    pub address: String,
    pub cookies: u128,
    pub last_click_time: SecondTimestamp,
    pub is_withdrawn: bool,
}

impl Gamer {
    pub fn new(address: String) -> Self {
        Self {
            address,
            cookies: 0,
            last_click_time: 0,
            is_withdrawn: false,
        }
    }
    pub fn claim(&mut self, claimed_cookies: u128) -> Result<u128> {
        self.cookies = self
            .cookies
            .checked_add(claimed_cookies)
            .ok_or(ExchangeError::Overflow)?;
        self.last_click_time = ic_cdk::api::time() / 1000_000_000;
        Ok(self.cookies)
    }
}

#[derive(Deserialize, Serialize, Clone, CandidType)]
pub struct GameAndGamer {
    pub game_duration: Seconds,
    pub game_start_time: SecondTimestamp,
    pub gamer_register_fee: Satoshi,
    pub claim_cooling_down: Seconds,
    pub cookie_amount_per_claim: u128,
    pub max_cookies: u128,
    pub claimed_cookies: u128,
    pub gamer: Option<Gamer>,
}

#[derive(Deserialize, Serialize, Clone, CandidType)]
pub struct RegisterInfo {
    pub untweaked_key: Pubkey,
    pub address: String,
    pub utxo: Option<Utxo>,
    pub register_fee: Satoshi,
    pub nonce: u64,
}
