use crate::*;
use candid::Nat;
use ic_cdk::api::call::RejectionCode;
use thiserror::Error;

pub type Result<R> = std::result::Result<R, ExchangeError>;

#[derive(Debug, Error, CandidType)]
pub enum ExchangeError {
    #[error("nat convert error: {0}")]
    NatConvertError(Nat),
    #[error("overflow")]
    Overflow,
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("invalid pool")]
    InvalidPool,
    #[error("invalid liquidity")]
    InvalidLiquidity,
    #[error("too small funds")]
    TooSmallFunds,
    #[error("lp not found")]
    LpNotFound,
    #[error("Fail to fetch rune info, RejectionCode {0:?}, msg {1} ")]
    FetchRuneIndexerError(RejectionCode, String),
    #[error("rune indexer result error {0}")]
    RuneIndexerResultError(String),
    #[error("invalid rune balance {0}")]
    ParseUtxoRuneBalanceError(String),
    #[error("invalid rune id")]
    InvalidRuneId,
    #[error("rune id not match, expect {0}, got {1}")]
    RuneIdNotMatch(String, String),
    #[error("Incorrect deposit rune balance, expect {0}, got {1}")]
    DepositRuneBalanceIncorrect(String, String),
    #[error("invalid txid: {0}")]
    InvalidTxid(String),
    #[error("invalid numeric")]
    InvalidNumeric,
    #[error("a pool with the given id already exists")]
    PoolAlreadyExists,
    #[error("the pool has not been initialized or has been removed")]
    EmptyPool,
    #[error("invalid input coin")]
    InvalidInput,
    #[error("couldn't derive a chain key for pool")]
    ChainKeyError,
    #[error("invalid psbt: {0}")]
    InvalidPsbt(String),
    #[error("invalid pool state: {0}")]
    InvalidState(String),
    #[error("Last state not found")]
    LastStateNotFound,
    #[error("invalid sign_psbt args: {0}")]
    InvalidSignPsbtArgs(String),
    #[error("pool state expired, current = {0}")]
    PoolStateExpired(u64),
    #[error("pool address not found")]
    PoolAddressNotFound,
    #[error("Cookie balance({0}) insufficient")]
    CookieBalanceInsufficient(u128),
    #[error("Game Not In InitState")]
    GameNotInInitState,
    #[error("Game Not End")]
    GameNotEnd,
    #[error("Game End")]
    GameEnd,
    #[error("Gamer Not Found, {0}")]
    GamerNotFound(Address),
    #[error("Gamer Withdraw Repeatedly, {0}")]
    GamerWithdrawRepeatedly(Address),
    #[error("Gamer Already Exist, {0}")]
    GamerAlreadyExist(Address),
    #[error("Gamer Cooling Down, {0} next claimable timestamp {1}")]
    GamerCoolingDown(Address, SecondTimestamp),
    #[error("Unrecoverable error")]
    Unrecoverable,
    #[error("Duplicate block, height: {0}, hash: {1}")]
    DuplicateBlock(u32, String),
    #[error("Invalid block, height: {0}, depth: {1}")]
    Recoverable(u32, u32),
    #[error("Reorg error: {0}")]
    ReorgError(#[from] reorg::ReorgError),
    #[error("{0}")]
    CustomError(String),
}
