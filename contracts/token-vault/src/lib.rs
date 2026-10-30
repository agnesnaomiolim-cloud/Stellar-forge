#![no_std]
// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

pub mod error;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, String,
};

pub const DEFAULT_MAX_GAS_LIMIT: u64 = 50_000_000;
pub const MAX_SUPPLY: i128 = i128::MAX;

#[contract]
pub struct TokenVault;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Paused,
    Decimals,
    Name,
    Symbol,
    TotalSupply,
    Balance(Address),
    Allowance(Address, Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowanceData {
    pub amount: i128,
    pub live_until_ledger: u32,
}

fn balance(env: &Env, account: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Balance(account.clone()))
        .unwrap_or(0)
}

fn set_balance(env: &Env, account: &Address, amount: i128) {
    assert!(amount >= 0, "negative balance");
    env.storage()
        .persistent()
        .set(&DataKey::Balance(account.clone()), &amount);
}

fn allowance(env: &Env, owner: &Address, spender: &Address) -> AllowanceData {
    env.storage()
        .persistent()
        .get(&DataKey::Allowance(owner.clone(), spender.clone()))
        .unwrap_or(AllowanceData {
            amount: 0,
            live_until_ledger: 0,
        })
}

fn require_amount(amount: i128) {
    assert!(amount >= 0, "amount must not be negative");
}

fn ensure_not_paused(env: &Env) {
    let paused = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    assert!(!paused, "contract is paused");
}


#[contractimpl]
impl TokenVault {
    /// Initializes the token vault with administrative control and token metadata.
    ///
    /// On success, stores the administrator, decimal precision, name, and symbol,
    /// sets the vault to unpaused, and initializes the total supply to zero.
    /// Initialization can only be performed once.
    ///
    /// # Arguments
    /// * `env` - The Soroban execution environment.
    /// * `admin` - The address granted administrative privileges; this address must authorize the call.
    /// * `decimals` - Number of decimal places used to represent token amounts.
    /// * `name` - Descriptive name of the vault token.

    pub fn initialize(env: Env, admin: Address, decimals: u32, name: String, symbol: String) {
        assert!(
            !env.storage().instance().has(&DataKey::Admin),
            "already initialized"
        );
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
    }

    pub fn admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized")
    }

    pub fn paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn set_paused(env: Env, paused: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &paused);
    }



    /// Mints new tokens to a designated recipient address.
    ///
    /// # Arguments
    /// * `env` - The Soroban execution environment.
    /// * `to` - The recipient address of the minted tokens.
    /// * `amount` - Amount of tokens to mint (must be non-negative).
    pub fn mint(env: Env, to: Address, amount: i128) {
        require_amount(amount);
        ensure_not_paused(&env);

        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0i128);
        let new_supply = supply.checked_add(amount).expect("supply overflow");
        assert!(new_supply <= MAX_SUPPLY, "exceeds max supply");

        let next = balance(&env, &to)
            .checked_add(amount)
            .expect("balance overflow");

        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &new_supply);
        set_balance(&env, &to, next);
        env.events().publish((symbol_short!("mint"), to), amount);
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        balance(&env, &id)
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        let data = allowance(&env, &from, &spender);
        if data.live_until_ledger != 0 && data.live_until_ledger < env.ledger().sequence() {
            0
        } else {
            data.amount
        }
    }

    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        live_until_ledger: u32,
    ) {
        require_amount(amount);
        from.require_auth();
        assert!(
            live_until_ledger == 0 || live_until_ledger >= env.ledger().sequence(),
            "invalid expiration"
        );
        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender.clone()),
            &AllowanceData {
                amount,
                live_until_ledger,
            },
        );
        env.events().publish(
            (symbol_short!("approve"), from, spender),
            (amount, live_until_ledger),
        );
    }

    /// Transfers tokens from caller's account to a destination address.
    ///
    /// Transfers of 0 amount are handled gracefully as a no-op per token specifications,
    /// leaving balances unchanged and emitting a transfer event.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        require_amount(amount);
        ensure_not_paused(&env);
        from.require_auth();

        let from_balance = balance(&env, &from);
        assert!(from_balance >= amount, "insufficient balance");


        set_balance(&env, &from, from_balance - amount);
        set_balance(
            &env,
            &to,
            balance(&env, &to)
                .checked_add(amount)
                .expect("balance overflow"),
        );
        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        require_amount(amount);
        ensure_not_paused(&env);

        spender.require_auth();
        let data = allowance(&env, &from, &spender);
        assert!(
            data.live_until_ledger == 0 || data.live_until_ledger >= env.ledger().sequence(),
            "allowance expired"
        );
        assert!(data.amount >= amount, "insufficient allowance");
        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender),
            &AllowanceData {
                amount: data.amount - amount,
                ..data
            },
        );
        let from_balance = balance(&env, &from);
        assert!(from_balance >= amount, "insufficient balance");

        set_balance(&env, &from, from_balance - amount);
        set_balance(
            &env,
            &to,
            balance(&env, &to)
                .checked_add(amount)
                .expect("balance overflow"),
        );
        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        require_amount(amount);
        ensure_not_paused(&env);
        from.require_auth();
        let current = balance(&env, &from);
        assert!(current >= amount, "insufficient balance");

        set_balance(&env, &from, current - amount);
        let supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(supply - amount));
        env.events().publish((symbol_short!("burn"), from), amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env, String};

    fn setup() -> (Env, TokenVaultClient<'static>, Address, Address, Address, Address) {
    fn setup() -> (
        Env,
        TokenVaultClient<'static>,
        Address,
        Address,
        Address,
        Address,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let spender = Address::generate(&env);

        let contract_id = env.register(TokenVault, ());
        let client = TokenVaultClient::new(&env, &contract_id);

        client.initialize(
            &admin,
            &7,
            &String::from_str(&env, "Token Vault"),
            &String::from_str(&env, "TVLT"),
        );

        (env, client, admin, alice, bob, spender)
    }

    #[test]
    #[should_panic(expected = "insufficient allowance")]
    fn test_transfer_from_exhausted_allowance_panics() {
        let (_env, client, _admin, alice, bob, spender) = setup();

        // Mint initial tokens to alice
        client.mint(&alice, &1000);
        assert_eq!(client.balance(&alice), 1000);

        // Alice approves spender for 100 tokens
        client.approve(&alice, &spender, &100, &0);
        assert_eq!(client.allowance(&alice, &spender), 100);

        // Spender transfers the full approved amount (100 tokens) to bob
        client.transfer_from(&spender, &alice, &bob, &100);
        assert_eq!(client.balance(&alice), 900);
        assert_eq!(client.balance(&bob), 100);

        // Verify that the allowance is now completely exhausted (0 remaining)
        assert_eq!(client.allowance(&alice, &spender), 0);

        // Attempting to transfer more tokens when allowance is exhausted must safely revert
        client.transfer_from(&spender, &alice, &bob, &1);
    }

    #[test]
    #[should_panic(expected = "insufficient allowance")]
    fn test_transfer_from_exceeds_remaining_allowance_panics() {
        let (_env, client, _admin, alice, bob, spender) = setup();

        client.mint(&alice, &1000);

        // Alice approves spender for 100 tokens
        client.approve(&alice, &spender, &100, &0);
        assert_eq!(client.allowance(&alice, &spender), 100);

        // Partial transfer of 60 tokens, leaving 40 allowance
        client.transfer_from(&spender, &alice, &bob, &60);
        assert_eq!(client.balance(&alice), 940);
        assert_eq!(client.balance(&bob), 60);
        assert_eq!(client.allowance(&alice, &spender), 40);

        // Attempting to transfer 50 tokens (exceeding remaining allowance of 40) must safely revert
        client.transfer_from(&spender, &alice, &bob, &50);
    }
}
