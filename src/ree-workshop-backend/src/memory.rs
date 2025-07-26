use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    Cell, DefaultMemoryImpl, StableBTreeMap,
};
use ree_types::{exchange_interfaces::NewBlockInfo, TxRecord, Txid};
use std::cell::RefCell;

use crate::state::ExchangeState;

pub type Memory = VirtualMemory<DefaultMemoryImpl>;

const STATE_MEMORY_ID: MemoryId = MemoryId::new(0);
const BLOCKS_MEMORY_ID: MemoryId = MemoryId::new(1);
const TX_RECORDS_MEMORY_ID: MemoryId = MemoryId::new(2);

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    static STATE: RefCell<Cell<Option<ExchangeState>, Memory>> = RefCell::new(Cell::init(MEMORY_MANAGER.with(|m| m.borrow().get(STATE_MEMORY_ID)), None).expect("state memory not initialized"));

    pub static BLOCKS: RefCell<StableBTreeMap<u32, NewBlockInfo, Memory>> = RefCell::new(
        StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(BLOCKS_MEMORY_ID))));

    pub static TX_RECORDS: RefCell<StableBTreeMap<(Txid, bool), TxRecord, Memory>> = RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(TX_RECORDS_MEMORY_ID))));
}

pub fn set_state(state: ExchangeState) {
    STATE.with(|c| {
        c.borrow_mut()
            .set(Some(state))
            .expect("Failed to set SETTINGS.")
    });
}

pub fn get_state() -> ExchangeState {
    STATE.with(|c| c.borrow().get().clone().unwrap())
}

pub fn mutate_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut ExchangeState) -> R,
{
    let mut state = get_state();
    let r = f(&mut state);
    set_state(state);
    r
}

pub fn read_state<F, R>(f: F) -> R
where
    F: FnOnce(&ExchangeState) -> R,
{
    let state = get_state();
    let r = f(&state);
    r
}
