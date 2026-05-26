#![no_std]
use emergency_guard::{EmergencyGuard, GuardError, PauseType};
use soroban_sdk::{
    contract, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal, Vec,
};

/// Storage key for pair registry.
/// Stored in **instance** storage because the factory is a singleton contract
/// and pair mappings are global state that should share the contract's TTL.
/// Using instance storage avoids per-entry persistent rent and reduces the
/// ledger footprint to a single entry per invocation.
#[contracttype]
pub enum DataKey {
    Pair(Address, Address),
}

#[contract]
pub struct LiquidityPoolFactory;

#[contractimpl]
impl LiquidityPoolFactory {
    /// Deploys a new Liquidity Pool contract for a unique pair of tokens.
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Address {
        if EmergencyGuard::is_paused(env.clone(), PauseType::MINT) {
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

        let salt = env
            .crypto()
            .sha256(&(token_0.clone(), token_1.clone()).to_xdr(&env));

        let deployed_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, soroban_sdk::Vec::<soroban_sdk::Val>::new(&env));

        let init_args = soroban_sdk::vec![
            &env,
            token_0.clone().into_val(&env),
            token_1.clone().into_val(&env)
        ];

        let _res: () = env.invoke_contract(
            &deployed_address,
            &soroban_sdk::Symbol::new(&env, "initialize"),
            init_args,
        );

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
