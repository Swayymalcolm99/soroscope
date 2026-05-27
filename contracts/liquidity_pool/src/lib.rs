#![no_std]

use emergency_guard::{EmergencyGuardTrait, GuardError, PauseType};
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};

#[cfg(test)]
mod fuzz_test;
#[cfg(test)]
mod test;

/// Errors returned by the `LiquidityPool` contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    InsufficientLiquidity = 2,
    SlippageExceeded = 3,
    InsufficientShares = 4,
    NotInitialized = 5,
    InsufficientBalance = 6,
    Unauthorized = 7,
    InsufficientAllowance = 8,
    InvalidFee = 9,
    OracleNotConfigured = 10,
    InvalidOraclePrice = 11,
    TimelockNotElapsed = 12,
    NoPendingFeeUpdate = 13,
    Paused = 14,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepositEvent {
    pub user: Address,
    pub amount_a: i128,
    pub amount_b: i128,
    pub shares_minted: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapEvent {
    pub user: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: i128,
    pub amount_out: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawEvent {
    pub user: Address,
    pub shares_burned: i128,
    pub amount_a: i128,
    pub amount_b: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurnEvent {
    pub user: Address,
    pub shares_burned: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeChangedEvent {
    pub admin: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeUpdateScheduledEvent {
    pub scheduled_by: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub volatility_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub token_a: Address,
    pub token_b: Address,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_shares: i128,
    pub fee_bps: i128,
    pub base_fee_bps: i128,
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardState {
    pub pause_state: u32,
    pub admins: Vec<Address>,
    pub signature_threshold: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    pub oracle: Address,
    pub last_price: i128,
    pub last_volatility_bps: i128,
    pub timelock_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFeeUpdate {
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub based_on_volatility_bps: i128,
}

#[derive(Clone)]
#[contracttype]
pub struct AllowanceDataKey {
    pub from: Address,
    pub spender: Address,
}

#[derive(Clone)]
#[contracttype]
pub struct AllowanceValue {
    pub amount: i128,
    pub expiration_ledger: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Pool,
    Guard,
    Oracle,
    PendingFeeUpdate,
    Balance(Address),
    Allowance(AllowanceDataKey),
}

pub const MAX_FEE_BPS: i128 = 100;
pub const DEFAULT_BASE_FEE_BPS: i128 = 30;
pub const DEFAULT_FEE_TIMELOCK_LEDGERS: u32 = 120;

pub const LOW_VOLATILITY_THRESHOLD_BPS: i128 = 100;
pub const MEDIUM_VOLATILITY_THRESHOLD_BPS: i128 = 250;
pub const HIGH_VOLATILITY_THRESHOLD_BPS: i128 = 500;

pub const LOW_VOLATILITY_FEE_BPS: i128 = 40;
pub const MEDIUM_VOLATILITY_FEE_BPS: i128 = 70;
pub const HIGH_VOLATILITY_FEE_BPS: i128 = 100;

#[soroban_sdk::contractclient(name = "PriceOracleClient")]
pub trait PriceOracle {
    fn latest_price(e: Env) -> i128;
}

fn sqrt(x: i128) -> i128 {
    if x == 0 {
        return 0;
    }

    let mut z = (x + 1) / 2;
    let mut y = x;

    while z < y {
        y = z;
        z = (x / z + z) / 2;
    }

    y
}

fn load_pool(e: &Env) -> Result<PoolState, Error> {
    e.storage()
        .instance()
        .get(&DataKey::Pool)
        .ok_or(Error::NotInitialized)
}

fn save_pool(e: &Env, pool: &PoolState) {
    e.storage().instance().set(&DataKey::Pool, pool);
}

fn load_guard(e: &Env) -> GuardState {
    e.storage()
        .instance()
        .get(&DataKey::Guard)
        .unwrap_or_else(|| GuardState {
            pause_state: 0,
            admins: Vec::new(e),
            signature_threshold: 0,
        })
}

fn save_guard(e: &Env, guard: &GuardState) {
    e.storage().instance().set(&DataKey::Guard, guard);
}

fn load_or_init_guard_from_pool(e: &Env, pool: &PoolState) -> GuardState {
    let guard = load_guard(e);
    if guard.signature_threshold == 0 && guard.admins.len() == 0 {
        GuardState {
            pause_state: guard.pause_state,
            admins: {
                let mut admins = Vec::new(e);
                admins.push_back(pool.admin.clone());
                admins
            },
            signature_threshold: 1,
        }
    } else {
        guard
    }
}

fn check_paused(pool: &PoolState, operation: u32, guard: &GuardState) -> Result<(), Error> {
    if guard.pause_state & operation != 0 {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

fn target_fee_from_volatility(base_fee_bps: i128, volatility_bps: i128) -> i128 {
    let dynamic_fee = if volatility_bps >= HIGH_VOLATILITY_THRESHOLD_BPS {
        HIGH_VOLATILITY_FEE_BPS
    } else if volatility_bps >= MEDIUM_VOLATILITY_THRESHOLD_BPS {
        MEDIUM_VOLATILITY_FEE_BPS
    } else if volatility_bps >= LOW_VOLATILITY_THRESHOLD_BPS {
        LOW_VOLATILITY_FEE_BPS
    } else {
        base_fee_bps
    };

    if dynamic_fee > MAX_FEE_BPS {
        MAX_FEE_BPS
    } else {
        dynamic_fee
    }
}

fn primary_admin(e: &Env) -> Result<Address, GuardError> {
    let guard = load_guard(e);
    if let Some(admin) = guard.admins.get(0) {
        Ok(admin)
    } else {
        let pool = load_pool(e).map_err(|_| GuardError::NotInitialized)?;
        Ok(pool.admin)
    }
}

fn check_multi_sig(e: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
    let guard = load_guard(e);
    if guard.signature_threshold == 0 {
        return Err(GuardError::NotInitialized);
    }

    if approvers.len() < guard.signature_threshold {
        return Err(GuardError::InsufficientSignatures);
    }

    let mut valid_approvers: u32 = 0;
    let mut seen = Vec::new(e);

    for addr in approvers.iter() {
        if seen.iter().any(|a| a == addr) {
            continue;
        }
        seen.push_back(addr.clone());

        if guard.admins.iter().any(|a| a == addr) {
            addr.require_auth();
            valid_approvers += 1;
        }
    }

    if valid_approvers < guard.signature_threshold {
        Err(GuardError::InsufficientSignatures)
    } else {
        Ok(())
    }
}

#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    pub fn initialize(
        e: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
    ) -> Result<(), Error> {
        if e.storage().instance().has(&DataKey::Pool) {
            return Err(Error::AlreadyInitialized);
        }

        save_pool(
            &e,
            &PoolState {
                token_a,
                token_b,
                reserve_a: 0,
                reserve_b: 0,
                total_shares: 0,
                fee_bps: DEFAULT_BASE_FEE_BPS,
                base_fee_bps: DEFAULT_BASE_FEE_BPS,
                admin: admin.clone(),
            },
        );

        save_guard(
            &e,
            &GuardState {
                pause_state: 0,
                admins: {
                    let mut admins = Vec::new(&e);
                    admins.push_back(admin);
                    admins
                },
                signature_threshold: 1,
            },
        );

        Ok(())
    }

    pub fn get_fee(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, PoolState>(&DataKey::Pool)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }

    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }

        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        let old_fee = pool.fee_bps;
        pool.fee_bps = fee_bps;
        pool.base_fee_bps = fee_bps;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "fee_changed"), pool.admin.clone()),
            FeeChangedEvent {
                admin: pool.admin,
                old_fee_bps: old_fee,
                new_fee_bps: fee_bps,
            },
        );

        Ok(())
    }

    pub fn configure_fee_oracle(
        e: Env,
        oracle: Address,
        base_fee_bps: i128,
        timelock_ledgers: u32,
    ) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&base_fee_bps) {
            return Err(Error::InvalidFee);
        }

        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        pool.base_fee_bps = base_fee_bps;
        save_pool(&e, &pool);

        e.storage().instance().set(
            &DataKey::Oracle,
            &OracleConfig {
                oracle,
                last_price: 0,
                last_volatility_bps: 0,
                timelock_ledgers,
            },
        );

        Ok(())
    }

    pub fn get_last_volatility_bps(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, OracleConfig>(&DataKey::Oracle)
            .map(|cfg| cfg.last_volatility_bps)
            .unwrap_or(0)
    }

    pub fn get_pending_fee_update(e: Env) -> Option<PendingFeeUpdate> {
        e.storage().instance().get(&DataKey::PendingFeeUpdate)
    }

    pub fn sync_fee_from_oracle(e: Env) -> Result<Option<PendingFeeUpdate>, Error> {
        let mut cfg: OracleConfig = e
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .ok_or(Error::OracleNotConfigured)?;

        let oracle_client = PriceOracleClient::new(&e, &cfg.oracle);
        let current_price = oracle_client.latest_price();
        if current_price <= 0 {
            return Err(Error::InvalidOraclePrice);
        }

        let previous_price = cfg.last_price;
        cfg.last_price = current_price;

        if previous_price <= 0 {
            cfg.last_volatility_bps = 0;
            e.storage().instance().set(&DataKey::Oracle, &cfg);
            return Ok(None);
        }

        let price_delta = if current_price >= previous_price {
            current_price - previous_price
        } else {
            previous_price - current_price
        };

        let volatility_bps = price_delta
            .checked_mul(10_000)
            .ok_or(Error::InvalidOraclePrice)?
            / previous_price;

        cfg.last_volatility_bps = volatility_bps;
        e.storage().instance().set(&DataKey::Oracle, &cfg);

        let pool = load_pool(&e)?;
        let target_fee = target_fee_from_volatility(pool.base_fee_bps, volatility_bps);
        if target_fee == pool.fee_bps {
            return Ok(None);
        }

        let execute_after = e.ledger().sequence().saturating_add(cfg.timelock_ledgers);
        let pending = PendingFeeUpdate {
            new_fee_bps: target_fee,
            executable_after_ledger: execute_after,
            based_on_volatility_bps: volatility_bps,
        };

        e.storage()
            .instance()
            .set(&DataKey::PendingFeeUpdate, &pending);

        let scheduled_by = e.current_contract_address();
        e.events().publish(
            (
                String::from_str(&e, "fee_update_scheduled"),
                scheduled_by.clone(),
            ),
            FeeUpdateScheduledEvent {
                scheduled_by,
                old_fee_bps: pool.fee_bps,
                new_fee_bps: target_fee,
                executable_after_ledger: execute_after,
                volatility_bps,
            },
        );

        Ok(Some(pending))
    }

    pub fn execute_fee_update(e: Env) -> Result<i128, Error> {
        let pending: PendingFeeUpdate = e
            .storage()
            .instance()
            .get(&DataKey::PendingFeeUpdate)
            .ok_or(Error::NoPendingFeeUpdate)?;

        if e.ledger().sequence() < pending.executable_after_ledger {
            return Err(Error::TimelockNotElapsed);
        }

        if !(0..=MAX_FEE_BPS).contains(&pending.new_fee_bps) {
            return Err(Error::InvalidFee);
        }

        let mut pool = load_pool(&e)?;
        let old_fee = pool.fee_bps;
        pool.fee_bps = pending.new_fee_bps;
        save_pool(&e, &pool);
        e.storage().instance().remove(&DataKey::PendingFeeUpdate);

        e.events().publish(
            (String::from_str(&e, "fee_changed"), pool.admin.clone()),
            FeeChangedEvent {
                admin: pool.admin,
                old_fee_bps: old_fee,
                new_fee_bps: pending.new_fee_bps,
            },
        );

        Ok(pending.new_fee_bps)
    }

    pub fn init_guard(e: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if threshold == 0 || threshold > admins.len() as u32 {
            return Err(GuardError::InvalidThreshold);
        }

        let first_admin = admins.get(0).ok_or(GuardError::NotInitialized)?;
        save_guard(
            &e,
            &GuardState {
                pause_state: load_guard(&e).pause_state,
                admins: admins.clone(),
                signature_threshold: threshold,
            },
        );

        if let Ok(mut pool) = load_pool(&e) {
            pool.admin = first_admin;
            save_pool(&e, &pool);
        }

        Ok(())
    }

    pub fn get_pause_state(e: Env) -> u32 {
        load_guard(&e).pause_state
    }

    pub fn set_pause_state(e: Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let admin = primary_admin(&e)?;
        admin.require_auth();

        let mut guard = load_guard(&e);
        if !guard.admins.iter().any(|a| a == admin) {
            return Err(GuardError::Unauthorized);
        }

        if paused {
            guard.pause_state |= operation;
        } else {
            guard.pause_state &= !operation;
        }

        save_guard(&e, &guard);
        Ok(())
    }

    pub fn emergency_pause_all(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;
        let mut guard = load_guard(&e);
        guard.pause_state = u32::MAX;
        save_guard(&e, &guard);
        Ok(())
    }

    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;
        let mut guard = load_guard(&e);
        guard.pause_state = 0;
        save_guard(&e, &guard);
        Ok(())
    }

    pub fn get_admins(e: Env) -> Vec<Address> {
        load_guard(&e).admins
    }

    pub fn get_threshold(e: Env) -> u32 {
        load_guard(&e).signature_threshold
    }

    pub fn is_admin(e: Env, addr: Address) -> bool {
        load_guard(&e).admins.iter().any(|a| a == addr)
    }

    pub fn add_admin(
        e: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;

        let mut guard = load_guard(&e);
        if !guard.admins.iter().any(|a| a == new_admin) {
            guard.admins.push_back(new_admin.clone());
            save_guard(&e, &guard);
        }

        Ok(())
    }

    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;

        let guard = load_guard(&e);
        if guard.admins.len() as u32 <= guard.signature_threshold {
            return Err(GuardError::InvalidThreshold);
        }

        let mut new_admins = Vec::new(&e);
        let mut found = false;
        for a in guard.admins.iter() {
            if a != admin {
                new_admins.push_back(a);
            } else {
                found = true;
            }
        }

        if !found {
            return Err(GuardError::AdminNotFound);
        }

        let mut updated_guard = guard;
        updated_guard.admins = new_admins;
        save_guard(&e, &updated_guard);

        if let Ok(mut pool) = load_pool(&e) {
            if let Some(first_admin) = updated_guard.admins.get(0) {
                pool.admin = first_admin;
                save_pool(&e, &pool);
            }
        }

        Ok(())
    }

    pub fn set_paused(e: Env, paused: bool) -> Result<(), Error> {
        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();

        let mut guard = load_or_init_guard_from_pool(&e, &pool);
        guard.pause_state = if paused { u32::MAX } else { 0 };
        save_guard(&e, &guard);

        Ok(())
    }

    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::emergency_pause_all(e, approvers).map_err(|_| Error::Unauthorized)
    }

    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::DEPOSIT, &guard)?;
        to.require_auth();

        let client_a = soroban_sdk::token::Client::new(&e, &pool.token_a);
        let client_b = soroban_sdk::token::Client::new(&e, &pool.token_b);
        client_a.transfer(&to, &e.current_contract_address(), &amount_a);
        client_b.transfer(&to, &e.current_contract_address(), &amount_b);

        let shares = if pool.total_shares == 0 {
            let product = amount_a
                .checked_mul(amount_b)
                .ok_or(Error::InsufficientLiquidity)?;
            sqrt(product)
        } else {
            let share_a = amount_a
                .checked_mul(pool.total_shares)
                .ok_or(Error::InsufficientLiquidity)?
                / pool.reserve_a;
            let share_b = amount_b
                .checked_mul(pool.total_shares)
                .ok_or(Error::InsufficientLiquidity)?
                / pool.reserve_b;
            if share_a < share_b {
                share_a
            } else {
                share_b
            }
        };

        let user_key = DataKey::Balance(to.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
        e.storage().persistent().set(&user_key, &(current + shares));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        pool.total_shares += shares;
        pool.reserve_a += amount_a;
        pool.reserve_b += amount_b;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "deposit"), to.clone()),
            DepositEvent {
                user: to,
                amount_a,
                amount_b,
                shares_minted: shares,
            },
        );

        Ok(shares)
    }

    pub fn swap(e: Env, to: Address, buy_a: bool, out: i128, in_max: i128) -> Result<i128, Error> {
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::SWAP, &guard)?;
        to.require_auth();

        let (reserve_in, reserve_out, token_in, token_out) = if buy_a {
            (
                pool.reserve_b,
                pool.reserve_a,
                pool.token_b.clone(),
                pool.token_a.clone(),
            )
        } else {
            (
                pool.reserve_a,
                pool.reserve_b,
                pool.token_a.clone(),
                pool.token_b.clone(),
            )
        };

        if out >= reserve_out {
            return Err(Error::InsufficientLiquidity);
        }

        let fee_scale = 10_000i128 - pool.fee_bps;
        let numerator = reserve_in
            .checked_mul(out)
            .ok_or(Error::InsufficientLiquidity)?
            .checked_mul(10_000)
            .ok_or(Error::InsufficientLiquidity)?;
        let denominator = (reserve_out - out)
            .checked_mul(fee_scale)
            .ok_or(Error::InsufficientLiquidity)?;
        let amount_in = (numerator / denominator) + 1;

        if amount_in > in_max {
            return Err(Error::SlippageExceeded);
        }

        soroban_sdk::token::Client::new(&e, &token_in).transfer(
            &to,
            &e.current_contract_address(),
            &amount_in,
        );
        soroban_sdk::token::Client::new(&e, &token_out).transfer(
            &e.current_contract_address(),
            &to,
            &out,
        );

        if buy_a {
            pool.reserve_a -= out;
            pool.reserve_b += amount_in;
        } else {
            pool.reserve_a += amount_in;
            pool.reserve_b -= out;
        }
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "swap"), to.clone()),
            SwapEvent {
                user: to,
                token_in,
                token_out,
                amount_in,
                amount_out: out,
            },
        );

        Ok(amount_in)
    }

    pub fn withdraw(e: Env, to: Address, share_amount: i128) -> Result<(i128, i128), Error> {
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::WITHDRAW, &guard)?;
        to.require_auth();

        let user_key = DataKey::Balance(to.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
        if share_amount > current {
            return Err(Error::InsufficientShares);
        }

        if pool.total_shares <= 0 {
            return Err(Error::InsufficientLiquidity);
        }

        let amount_a = share_amount * pool.reserve_a / pool.total_shares;
        let amount_b = share_amount * pool.reserve_b / pool.total_shares;

        e.storage()
            .persistent()
            .set(&user_key, &(current - share_amount));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        pool.total_shares -= share_amount;
        pool.reserve_a -= amount_a;
        pool.reserve_b -= amount_b;
        save_pool(&e, &pool);

        soroban_sdk::token::Client::new(&e, &pool.token_a).transfer(
            &e.current_contract_address(),
            &to,
            &amount_a,
        );
        soroban_sdk::token::Client::new(&e, &pool.token_b).transfer(
            &e.current_contract_address(),
            &to,
            &amount_b,
        );

        e.events().publish(
            (String::from_str(&e, "withdraw"), to.clone()),
            WithdrawEvent {
                user: to,
                shares_burned: share_amount,
                amount_a,
                amount_b,
            },
        );

        Ok((amount_a, amount_b))
    }

    pub fn burn(e: Env, from: Address, amount: i128) -> Result<(), Error> {
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::BURN, &guard)?;
        from.require_auth();

        let user_key = DataKey::Balance(from.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
        if amount > current {
            return Err(Error::InsufficientShares);
        }

        e.storage().persistent().set(&user_key, &(current - amount));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        pool.total_shares -= amount;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "burn"), from.clone()),
            BurnEvent {
                user: from,
                shares_burned: amount,
            },
        );

        Ok(())
    }

    pub fn name(e: Env) -> String {
        String::from_str(&e, "Liquidity Pool Share")
    }

    pub fn symbol(e: Env) -> String {
        String::from_str(&e, "LPS")
    }

    pub fn decimals(_e: Env) -> u32 {
        7
    }

    pub fn balance(e: Env, id: Address) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn total_supply(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, PoolState>(&DataKey::Pool)
            .map(|p| p.total_shares)
            .unwrap_or(0)
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();

        let from_key = DataKey::Balance(from.clone());
        let to_key = DataKey::Balance(to.clone());

        let from_balance: i128 = e.storage().persistent().get(&from_key).unwrap_or(0);
        if from_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        e.storage()
            .persistent()
            .set(&from_key, &(from_balance - amount));
        e.storage().persistent().extend_ttl(&from_key, 100, 100);

        let to_balance: i128 = e.storage().persistent().get(&to_key).unwrap_or(0);
        e.storage()
            .persistent()
            .set(&to_key, &(to_balance + amount));
        e.storage().persistent().extend_ttl(&to_key, 100, 100);

        Ok(())
    }

    pub fn approve(
        e: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) -> Result<(), Error> {
        from.require_auth();

        let key = DataKey::Allowance(AllowanceDataKey {
            from: from.clone(),
            spender: spender.clone(),
        });
        e.storage().persistent().set(
            &key,
            &AllowanceValue {
                amount,
                expiration_ledger,
            },
        );
        e.storage().persistent().extend_ttl(&key, 100, 100);

        Ok(())
    }

    pub fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        let key = DataKey::Allowance(AllowanceDataKey { from, spender });
        match e.storage().persistent().get::<_, AllowanceValue>(&key) {
            Some(a) if e.ledger().sequence() <= a.expiration_ledger => a.amount,
            _ => 0,
        }
    }

    pub fn transfer_from(
        e: Env,
        spender: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), Error> {
        spender.require_auth();

        let current_allowance = Self::allowance(e.clone(), from.clone(), spender.clone());
        if current_allowance < amount {
            return Err(Error::InsufficientAllowance);
        }

        let new_allowance = current_allowance - amount;
        let key = DataKey::Allowance(AllowanceDataKey {
            from: from.clone(),
            spender: spender.clone(),
        });

        if new_allowance > 0 {
            let current_val = e
                .storage()
                .persistent()
                .get::<_, AllowanceValue>(&key)
                .unwrap();
            e.storage().persistent().set(
                &key,
                &AllowanceValue {
                    amount: new_allowance,
                    expiration_ledger: current_val.expiration_ledger,
                },
            );
            e.storage().persistent().extend_ttl(&key, 100, 100);
        } else {
            e.storage().persistent().remove(&key);
        }

        Self::transfer(e, from, to, amount)
    }
}

impl EmergencyGuardTrait for LiquidityPool {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        let guard = load_guard(env);
        if guard.pause_state & operation != 0 {
            Err(GuardError::Paused)
        } else {
            Ok(())
        }
    }

    fn get_pause_state(env: &Env) -> u32 {
        load_guard(env).pause_state
    }

    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let admin = primary_admin(env)?;
        admin.require_auth();

        let mut guard = load_guard(env);
        if !guard.admins.iter().any(|a| a == admin) {
            return Err(GuardError::Unauthorized);
        }

        if paused {
            guard.pause_state |= operation;
        } else {
            guard.pause_state &= !operation;
        }
        save_guard(env, &guard);

        Ok(())
    }

    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;
        let mut guard = load_guard(env);
        guard.pause_state = u32::MAX;
        save_guard(env, &guard);
        Ok(())
    }

    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;
        let mut guard = load_guard(env);
        guard.pause_state = 0;
        save_guard(env, &guard);
        Ok(())
    }

    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if threshold == 0 || threshold > admins.len() as u32 {
            return Err(GuardError::InvalidThreshold);
        }

        let first_admin = admins.get(0).ok_or(GuardError::NotInitialized)?;
        save_guard(
            env,
            &GuardState {
                pause_state: load_guard(env).pause_state,
                admins: admins.clone(),
                signature_threshold: threshold,
            },
        );

        if let Ok(mut pool) = load_pool(env) {
            pool.admin = first_admin;
            save_pool(env, &pool);
        }

        Ok(())
    }

    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;

        let mut guard = load_guard(env);
        if !guard.admins.iter().any(|a| a == new_admin) {
            guard.admins.push_back(new_admin);
            save_guard(env, &guard);
        }

        Ok(())
    }

    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;

        let guard = load_guard(env);
        if guard.admins.len() as u32 <= guard.signature_threshold {
            return Err(GuardError::InvalidThreshold);
        }

        let mut new_admins = Vec::new(env);
        let mut found = false;
        for a in guard.admins.iter() {
            if a != admin {
                new_admins.push_back(a);
            } else {
                found = true;
            }
        }

        if !found {
            return Err(GuardError::AdminNotFound);
        }

        save_guard(
            env,
            &GuardState {
                pause_state: guard.pause_state,
                admins: new_admins,
                signature_threshold: guard.signature_threshold,
            },
        );

        if let Ok(mut pool) = load_pool(env) {
            let updated_guard = load_guard(env);
            if let Some(first_admin) = updated_guard.admins.get(0) {
                pool.admin = first_admin;
                save_pool(env, &pool);
            }
        }

        Ok(())
    }

    fn get_admins(env: &Env) -> Vec<Address> {
        load_guard(env).admins
    }

    fn get_threshold(env: &Env) -> u32 {
        load_guard(env).signature_threshold
    }

    fn is_admin(env: &Env, addr: &Address) -> bool {
        load_guard(env).admins.iter().any(|a| a == *addr)
    }
}
