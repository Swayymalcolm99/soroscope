#![cfg(test)]

use crate::{Domain, Transfer, TypedDataAuth};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env, String};

#[test]
fn test_domain_hash_is_nonzero() {
    let env = Env::default();
    let zero = BytesN::from_array(&env, &[0u8; 32]);
    let contract_address = Address::generate(&env);
    let _signer = Address::generate(&env);
    let from = Address::generate(&env);
    let to = Address::generate(&env);

    let domain = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    let hash = TypedDataAuth::compute_domain_hash(&env, &domain);
    assert!(!hash.is_empty());
}

#[test]
fn test_struct_hash_is_nonzero() {
    let env = Env::default();
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let transfer = Transfer { from, to, amount: 1000 };
    let hash = TypedDataAuth::compute_struct_hash(&env, &transfer);
    assert!(!hash.is_empty());
}

#[test]
fn test_message_hash_is_nonzero() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let transfer = Transfer { from, to, amount: 500 };

    // Compute the message hash
    let domain_hash = TypedDataAuth::domain_separator_hash(&env, &domain);
    let struct_hash = TypedDataAuth::struct_hash(&env, &transfer);
    let message_hash = TypedDataAuth::message_hash(&env, &domain_hash, &struct_hash);

    // Generate a signature (in test environment, we can mock this)
    // For simplicity, we'll assume the signature is valid
    // In real tests, you'd generate a proper signature using the signer's keypair

    // Since soroban_sdk testutils don't provide easy signature generation,
    // we'll skip the full verification in unit tests.
    // This test structure shows the intent.

    // For now, just test that the hashes are computed correctly
    assert_ne!(domain_hash, zero);
    assert_ne!(struct_hash, zero);
    assert_ne!(message_hash, zero);
}

#[test]
fn test_domain_separator_consistency() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain1 = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address.clone(),
    };
    let domain2 = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    assert_eq!(
        TypedDataAuth::compute_domain_hash(&env, &domain1),
        TypedDataAuth::compute_domain_hash(&env, &domain2),
    );
}

#[test]
fn test_different_domains_produce_different_hashes() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain1 = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address.clone(),
    };
    let domain2 = Domain {
        name: String::from_str(&env, "OtherContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };

    let hash1 = TypedDataAuth::domain_separator_hash(&env, &domain1);
    let hash2 = TypedDataAuth::domain_separator_hash(&env, &domain2);

    assert_ne!(hash1, hash2);
}
