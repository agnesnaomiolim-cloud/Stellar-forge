#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

use crate::{RentManager, RentManagerClient};

fn setup() -> (Env, Address, RentManagerClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let asset_id = env.register_stellar_asset_contract_v2(admin.clone());
    let asset = asset_id.address();
    let contract_id = env.register(RentManager, ());
    let client = RentManagerClient::new(&env, &contract_id);
    client.initialize(&admin, &asset);
    (env, contract_id, client, admin, asset)
}

fn fund_and_approve(
    env: &Env,
    contract_id: &Address,
    asset: &Address,
    owner: &Address,
    amount: i128,
) {
    let token_admin = StellarAssetClient::new(env, asset);
    let token = TokenClient::new(env, asset);
    token_admin.mint(owner, &amount);
    token.approve(
        owner,
        contract_id,
        &amount,
        &(env.ledger().sequence() + 1_000),
    );
}

#[test]
fn deposit_and_withdraw_preserve_backed_funds() {
    let (env, contract_id, client, _admin, asset) = setup();
    let owner = Address::generate(&env);
    fund_and_approve(&env, &contract_id, &asset, &owner, 500);

    client.deposit(&owner, &300);
    assert_eq!(client.balance(&owner), 300);
    assert_eq!(client.total_deposits().unwrap(), 300);

    client.withdraw(&owner, &125);
    assert_eq!(client.balance(&owner), 175);
    assert_eq!(client.total_deposits().unwrap(), 175);
    assert_eq!(TokenClient::new(&env, &asset).balance(&owner), 325);
}

#[test]
fn restore_preserves_balance_after_a_ledger_jump() {
    let (env, contract_id, client, _admin, asset) = setup();
    let owner = Address::generate(&env);
    fund_and_approve(&env, &contract_id, &asset, &owner, 500);
    client.deposit(&owner, &500);

    env.ledger().set_sequence_number(env.ledger().sequence() + 10_000);
    assert_eq!(client.restore(&owner).unwrap(), 500);
    assert_eq!(client.balance(&owner), 500);
    assert_eq!(client.total_deposits().unwrap(), 500);
}

#[test]
fn missing_balance_is_not_claimed_as_restored() {
    let (env, _contract_id, client, _admin, _asset) = setup();
    let owner = Address::generate(&env);

    assert_eq!(
        client.try_restore(&owner),
        Err(Ok(crate::RentManagerError::NoRestorableBalance))
    );
}
