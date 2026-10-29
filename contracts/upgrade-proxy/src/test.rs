//! Initialization and governance tests for the Upgrade Proxy contract.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{error::ProxyError, UpgradeProxyContract, UpgradeProxyContractClient};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Spin up a fresh Soroban test environment and deploy the UpgradeProxyContract.
fn setup() -> (Env, UpgradeProxyContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, UpgradeProxyContract);
    let client = UpgradeProxyContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let dao_council = Address::generate(&env);

    (env, client, admin, dao_council)
}

// ---------------------------------------------------------------------------
// Initialization tests
// ---------------------------------------------------------------------------

#[test]
fn test_initialize_stores_admin_and_dao_council() {
    let (_env, client, admin, dao_council) = setup();

    client.initialize(&admin, &dao_council);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.dao_council(), dao_council);
    assert!(client.pending_upgrade().is_none());
    assert_eq!(client.timelock_remaining(), 0);
}

#[test]
fn test_double_initialize_rejected() {
    let (env, client, admin, dao_council) = setup();

    client.initialize(&admin, &dao_council);

    let other_admin = Address::generate(&env);
    let other_council = Address::generate(&env);
    let result = client.try_initialize(&other_admin, &other_council);

    assert_eq!(result, Err(Ok(ProxyError::AlreadyInitialized)));
    assert_eq!(client.admin(), admin);
    assert_eq!(client.dao_council(), dao_council);
}

#[test]
fn test_queries_before_initialization_return_not_initialized() {
    let (_env, client, _admin, _dao_council) = setup();

    assert_eq!(client.try_admin(), Err(Ok(ProxyError::NotInitialized)));
    assert_eq!(
        client.try_dao_council(),
        Err(Ok(ProxyError::NotInitialized))
    );
}

#[test]
fn test_initialize_requires_admin_auth() {
    let env = Env::default();
    let contract_id = env.register_contract(None, UpgradeProxyContract);
    let client = UpgradeProxyContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let dao_council = Address::generate(&env);

    // Without mock_all_auths(), initialize must fail when admin has not authorized.
    let result = client.try_initialize(&admin, &dao_council);
    assert!(result.is_err());
    assert_eq!(client.try_admin(), Err(Ok(ProxyError::NotInitialized)));
}
