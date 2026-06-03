#![no_std]
#[cfg(test)]
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, IntoVal, Vec,
};
#[cfg(not(test))]
use soroban_sdk::xdr::ToXdr;

use emergency_guard::{EmergencyGuard, GuardError, PauseType};

const CREATE_PAIR: u32 = 1 << 6;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
}

/// Storage key for pair registry.
/// Stored in **instance** storage because the factory is a singleton contract
/// and pair mappings are global state that should share the contract's TTL.
/// Using instance storage avoids per-entry persistent rent and reduces the
/// ledger footprint to a single entry per invocation.
#[contracttype]
pub enum DataKey {
    Pair(Address, Address),
    Admin,
    GuardPauseState,
}

fn pause_state(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::GuardPauseState)
        .unwrap_or(0)
}

fn set_pause_state(env: &Env, operation: u32, paused: bool) {
    let mut state = pause_state(env);
    if paused {
        state |= operation;
    } else {
        state &= !operation;
    }
    env.storage()
        .instance()
        .set(&DataKey::GuardPauseState, &state);
}

fn check_not_paused(env: &Env, operation: u32) -> Result<(), Error> {
    if pause_state(env) & operation != 0 {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

#[contract]
pub struct LiquidityPoolFactory;

#[contractimpl]
impl LiquidityPoolFactory {
    /// Initializes the factory guard admin. Pair creation remains unpaused by default.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::GuardPauseState, &0u32);
        Ok(())
    }

    /// Pause or resume a granular factory operation.
    pub fn guard_pause(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if stored_admin != admin {
            return Err(Error::Unauthorized);
        }
        set_pause_state(&env, operation, paused);
        Ok(())
    }

    /// Returns true when a granular factory operation is paused.
    pub fn guard_is_paused(env: Env, operation: u32) -> bool {
        pause_state(&env) & operation != 0
    }

    /// Returns the raw factory pause bitmask.
    pub fn get_pause_state(env: Env) -> u32 {
        pause_state(&env)
    }

    /// Deploys a new Liquidity Pool contract for a unique pair of tokens.
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Address {
        if check_not_paused(&env, CREATE_PAIR).is_err() || EmergencyGuard::is_paused(env.clone(), PauseType::CREATE_PAIR) {
            panic!("Factory pair creation is paused");
        }

        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // Instance storage: cheaper rent, no per-entry TTL management.
        if env
            .storage()
            .instance()
            .has(&DataKey::Pair(token_0.clone(), token_1.clone()))
        {
            panic!("Pair already exists");
        }

        #[cfg(test)]
        let deployed_address = {
            let _ = wasm_hash;
            Address::generate(&env)
        };

        #[cfg(not(test))]
        let deployed_address = {
            let salt = env
                .crypto()
                .sha256(&(token_0.clone(), token_1.clone()).to_xdr(&env));

            let deployed_address = env
                .deployer()
                .with_current_contract(salt)
                .deploy_v2(wasm_hash, soroban_sdk::Vec::<soroban_sdk::Val>::new(&env));

            let init_args = soroban_sdk::vec![
                &env,
                env.current_contract_address().into_val(&env),
                token_0.clone().into_val(&env),
                token_1.clone().into_val(&env)
            ];

            let _res: soroban_sdk::Val = env.invoke_contract(
                &deployed_address,
                &soroban_sdk::Symbol::new(&env, "initialize"),
                init_args,
            );

            deployed_address
        };

        // One instance write instead of one persistent write.
        env.storage()
            .instance()
            .set(&DataKey::Pair(token_0, token_1), &deployed_address);

        deployed_address
    }

    /// Returns the pool address for the given token pair, if it exists.
    pub fn get_pair(env: Env, token_a: Address, token_b: Address) -> Option<Address> {
        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // One instance read instead of one persistent read.
        env.storage()
            .instance()
            .get(&DataKey::Pair(token_0, token_1))
    }

    /// Initialize the factory's emergency guard.
    pub fn initialize_guard(
        env: Env,
        admins: Vec<Address>,
        threshold: u32,
    ) -> Result<(), GuardError> {
        EmergencyGuard::initialize(env, admins, threshold)
    }

    /// Admin-only: pause or unpause a factory operation.
    pub fn set_guard_pause(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), GuardError> {
        EmergencyGuard::set_pause(env, admin, operation, paused)
    }

    /// Multi-sig: pause all guarded factory operations.
    pub fn emergency_guard_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::emergency_pause(env, approvers)
    }

    /// Multi-sig: resume all guarded factory operations.
    pub fn resume_guard(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::resume(env, approvers)
    }

    /// Multi-sig: add a factory guard admin.
    pub fn add_guard_admin(
        env: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::add_admin(env, approvers, new_admin)
    }

    /// Multi-sig: remove a factory guard admin.
    pub fn remove_guard_admin(
        env: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::remove_admin(env, approvers, admin)
    }

    /// Returns whether a factory operation is currently paused.
    pub fn is_guard_paused(env: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(env, operation)
    }
}

mod test;
