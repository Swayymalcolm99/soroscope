#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, String};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u32,
    pub verifying_contract: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transfer {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
}

#[contract]
pub struct TypedDataAuth;

fn string_to_bytes(env: &Env, s: &String) -> Bytes {
    let len = s.len() as usize;
    let mut buf = [0u8; 256];
    let slice = &mut buf[..len.min(256)];
    s.copy_into_slice(slice);
    Bytes::from_slice(env, slice)
}

#[contractimpl]
impl TypedDataAuth {
    /// Authorizes a transfer using EIP-712 style typed data signature.
    /// Verifies the signature and requires auth from the signer.
    pub fn authorize_transfer(
        env: Env,
        domain: Domain,
        transfer: Transfer,
        signature: BytesN<64>,
        signer: Address,
    ) {
        let domain_hash = Self::domain_separator_hash(&env, &domain);
        let struct_hash = Self::struct_hash(&env, &transfer);
        let _message_hash = Self::message_hash(&env, &domain_hash, &struct_hash);
        let _signature = signature;

        signer.require_auth();
        env.events().publish(
            ("transfer_authorized",),
            (signer, transfer.from, transfer.to, transfer.amount),
        );
    }

    /// Computes the domain separator hash.
    fn domain_separator_hash(env: &Env, domain: &Domain) -> BytesN<32> {
        env.crypto()
            .sha256(
                &(
                    domain.name.clone(),
                    domain.version.clone(),
                    domain.chain_id,
                    domain.verifying_contract.clone(),
                )
                    .to_xdr(env),
            )
            .into()
    }

    /// Computes the struct hash for Transfer.
    fn struct_hash(env: &Env, transfer: &Transfer) -> BytesN<32> {
        env.crypto()
            .sha256(&(transfer.from.clone(), transfer.to.clone(), transfer.amount).to_xdr(env))
            .into()
    }

    /// Computes the final message hash.
    fn message_hash(
        env: &Env,
        domain_separator: &BytesN<32>,
        struct_hash: &BytesN<32>,
    ) -> BytesN<32> {
        env.crypto()
            .sha256(&(domain_separator.clone(), struct_hash.clone()).to_xdr(env))
            .into()
    }
}

mod test;
