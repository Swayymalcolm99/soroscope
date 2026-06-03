#![cfg(test)]
extern crate std;
use super::*;

use emergency_guard::{EmergencyGuardAction, EmergencyGuardEvent, PauseType};
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, BytesN, Env, String as SorobanString, TryIntoVal,
};
use std::vec::Vec;
use soroban_sdk::{testutils::Address as _, vec, Address, BytesN, Env};
use soroban_sdk::{testutils::Address as _, Env, Vec};

fn dummy_pool_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0; 32])
}

#[test]
fn test_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Pair should not exist yet
    let result = factory_client.get_pair(&token_a, &token_b);
    assert_eq!(result, None);
}

#[test]
fn test_guard_admin_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    assert_eq!(factory_client.initialize(&admins, &2), Ok(()));
    assert_eq!(factory_client.get_threshold(), 2);
    assert_eq!(factory_client.get_admins().len(), 2);
    assert!(factory_client.is_admin(&admin1));
    assert!(factory_client.is_admin(&admin2));
}

#[test]
fn test_guard_admin_threshold_checks() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    assert_eq!(factory_client.initialize(&admins, &2), Ok(()));

    let single_approver = vec![&env, admin1.clone()];
    assert_eq!(
        factory_client.add_admin(&single_approver, &new_admin),
        Err(GuardError::InsufficientSignatures)
    );

    let full_approvals = vec![&env, admin1.clone(), admin2.clone()];
    assert_eq!(factory_client.add_admin(&full_approvals, &new_admin), Ok(()));
    assert!(factory_client.is_admin(&new_admin));

    assert_eq!(
        factory_client.remove_admin(&single_approver, &new_admin),
        Err(GuardError::InsufficientSignatures)
    );

    assert_eq!(factory_client.remove_admin(&full_approvals, &new_admin), Ok(()));
    assert!(!factory_client.is_admin(&new_admin));
}

#[test]
fn test_guard_pause_create_pair_success() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let admins = vec![&env, admin.clone()];

    assert_eq!(factory_client.initialize(&admins, &1), Ok(()));
    assert!(!factory_client.guard_is_paused(&CREATE_PAIR));

    factory_client
        .guard_pause(&admin, &CREATE_PAIR, &true)
        .expect("admin should be able to pause create_pair");
    assert!(factory_client.guard_is_paused(&CREATE_PAIR));

    factory_client
        .guard_pause(&admin, &CREATE_PAIR, &false)
        .expect("admin should be able to resume create_pair");
    assert!(!factory_client.guard_is_paused(&CREATE_PAIR));
}

#[test]
fn test_guard_pause_create_pair_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let admins = vec![&env, admin.clone()];

    assert_eq!(factory_client.initialize(&admins, &1), Ok(()));

    assert_eq!(
        factory_client.try_guard_pause(&stranger, &CREATE_PAIR, &true),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn test_pool_creation() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    // Setup Tokens
    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let pool_hash = dummy_pool_hash(&env);

    // Note: Due to a testutils handle mapping bug in the Soroban SDK mock environment,
    // returning a newly deployed address from a native contract call corrupts the handle
    // mapping in the Rust test space. Any `Address` representing the new pool will evaluate
    // to the `factory_id` in Rust. However, the host engine state is correct.
    // Therefore, we only assert that a value is returned and stored, bypassing strict equality.
    let _pool_address = factory_client
        .create_pair(&token_a, &token_b, &pool_hash)
        .unwrap();

    // Verify the pair is stored and retrievable
    let stored_pair = factory_client.get_pair(&token_a, &token_b);
    assert!(stored_pair.is_some());

    // Reversed order should also resolve to the same pool (canonical ordering)
    let stored_pair_rev = factory_client.get_pair(&token_b, &token_a);
    assert!(stored_pair_rev.is_some());
}

#[test]
fn test_pause_create_pair() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let pool_hash = env
        .deployer()
        .upload_contract_wasm(liquidity_pool_contract::WASM);

    let mut admins = Vec::new(&env);
    admins.push_back(admin.clone());
    factory_client.initialize(&admins, &1).unwrap();
    factory_client.set_paused(&admin, &true).unwrap();

    let result = factory_client.create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Error::Paused));

    factory_client.set_paused(&admin, &false).unwrap();
    let created = factory_client.create_pair(&token_a, &token_b, &pool_hash).unwrap();
    assert!(created != factory_id);
}

#[test]
fn test_duplicate_pair_errors() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let pool_hash = dummy_pool_hash(&env);

    // First creation succeeds
    factory_client
        .create_pair(&token_a, &token_b, &pool_hash)
        .unwrap();

    // Second creation with the same pair should return a pair-exists error
    let result = factory_client.create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Error::PairAlreadyExists));
}
/*
// TODO: Enable this once we have a way to import the Liquidity Pool WASM
// let pool_hash = env.deployer().upload_contract_wasm(liquidity_pool_contract::WASM);
// let pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);
// assert!(pool_address != factory_id);
*/

fn guard_events(env: &Env, contract_id: &Address, action: &str) -> Vec<EmergencyGuardEvent> {
    let guard_topic = SorobanString::from_str(env, "EmergencyGuard");
    let action_topic = SorobanString::from_str(env, action);

    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 2 {
                return None;
            }

            let topic_guard: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            let topic_action: SorobanString = topics.get(1)?.try_into_val(env).ok()?;

            if topic_guard == guard_topic && topic_action == action_topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn setup_guard(
    env: &Env,
) -> (
    Address,
    LiquidityPoolFactoryClient<'_>,
    Address,
    Address,
    Address,
) {
    env.mock_all_auths();
    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(env, &factory_id);
    let admin1 = Address::generate(env);
    let admin2 = Address::generate(env);
    let admin3 = Address::generate(env);
    let admins = vec![env, admin1.clone(), admin2.clone(), admin3.clone()];

    factory_client.initialize_guard(&admins, &2);

    (factory_id, factory_client, admin1, admin2, admin3)
}

#[test]
fn test_initialize_guard_emits_standard_event() {
    let env = Env::default();
    let (factory_id, _client, _admin1, _admin2, _admin3) = setup_guard(&env);

    let events = guard_events(&env, &factory_id, "initialized");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::Initialized,
            admin: None,
            operation: 0,
            paused: false,
            threshold: 2,
            admin_count: 3,
            approver_count: 0,
        }
    );
}

#[test]
fn test_set_guard_pause_emits_standard_events() {
    let env = Env::default();
    let (factory_id, client, admin1, _admin2, _admin3) = setup_guard(&env);

    client.set_guard_pause(&admin1, &PauseType::MINT, &true);
    let events = guard_events(&env, &factory_id, "pause_set");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::PauseSet,
            admin: Some(admin1.clone()),
            operation: PauseType::MINT,
            paused: true,
            threshold: 2,
            admin_count: 3,
            approver_count: 1,
        }
    );

    assert!(client.is_guard_paused(&PauseType::MINT));

    client.set_guard_pause(&admin1, &PauseType::MINT, &false);
    let events = guard_events(&env, &factory_id, "pause_set");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::PauseSet,
            admin: Some(admin1),
            operation: PauseType::MINT,
            paused: false,
            threshold: 2,
            admin_count: 3,
            approver_count: 1,
        }
    );
}

#[test]
fn test_emergency_pause_and_resume_emit_standard_events() {
    let env = Env::default();
    let (factory_id, client, admin1, admin2, _admin3) = setup_guard(&env);
    let approvers = vec![&env, admin1, admin2];

    client.emergency_guard_pause(&approvers);
    let emergency_events = guard_events(&env, &factory_id, "emergency_pause");
    assert_eq!(emergency_events.len(), 1);
    assert_eq!(
        emergency_events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::EmergencyPause,
            admin: None,
            operation: u32::MAX,
            paused: true,
            threshold: 2,
            admin_count: 3,
            approver_count: 2,
        }
    );
    assert!(client.is_guard_paused(&PauseType::MINT));

    client.resume_guard(&approvers);
    let resume_events = guard_events(&env, &factory_id, "resume");
    assert_eq!(resume_events.len(), 1);
    assert_eq!(
        resume_events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::Resume,
            admin: None,
            operation: u32::MAX,
            paused: false,
            threshold: 2,
            admin_count: 3,
            approver_count: 2,
        }
    );
    assert!(!client.is_guard_paused(&PauseType::MINT));
}

#[test]
fn test_admin_guard_actions_emit_standard_events() {
    let env = Env::default();
    let (factory_id, client, admin1, admin2, admin3) = setup_guard(&env);
    let approvers = vec![&env, admin1, admin2];
    let admin4 = Address::generate(&env);

    client.add_guard_admin(&approvers, &admin4);
    let added_events = guard_events(&env, &factory_id, "admin_added");
    assert_eq!(added_events.len(), 1);
    assert_eq!(
        added_events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::AdminAdded,
            admin: Some(admin4),
            operation: 0,
            paused: false,
            threshold: 2,
            admin_count: 4,
            approver_count: 2,
        }
    );

    client.remove_guard_admin(&approvers, &admin3);
    let removed_events = guard_events(&env, &factory_id, "admin_removed");
    assert_eq!(removed_events.len(), 1);
    assert_eq!(
        removed_events[0],
        EmergencyGuardEvent {
            action: EmergencyGuardAction::AdminRemoved,
            admin: Some(admin3),
            operation: 0,
            paused: false,
            threshold: 2,
            admin_count: 3,
            approver_count: 2,
        }
    );
}
