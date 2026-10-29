// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};

fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(10);
    let admin = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let id = env.register(Governance, (Config {
        admin, proposal_threshold: 10, quorum_bps: 5_000, voting_ledgers: 5,
    },));
    (env, id, a, b, c)
}

#[test]
fn transitive_delegation_and_redelegation_preserve_weight() {
    let (env, id, a, b, c) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &20);
    client.mint(&b, &30);
    client.mint(&c, &50);
    client.delegate(&a, &Some(b.clone()));
    client.delegate(&b, &Some(c.clone()));
    env.ledger().set_sequence_number(11);
    assert_eq!(client.votes_at(&a, &10), 0);
    assert_eq!(client.votes_at(&b, &10), 0);
    assert_eq!(client.votes_at(&c, &10), 100);
    let proposal = client.propose(&c, &BytesN::from_array(&env, &[1; 32]));
    env.ledger().set_sequence_number(12);
    client.delegate(&b, &None);
    assert_eq!(client.vote(&proposal, &c, &Choice::For), 100);
    assert_eq!(client.try_vote(&proposal, &b, &Choice::Against), Err(Ok(Error::NoVotingPower)));
    assert_eq!(client.try_vote(&proposal, &c, &Choice::Abstain), Err(Ok(Error::AlreadyVoted)));
    env.ledger().set_sequence_number(13);
    assert_eq!(client.votes_at(&b, &12), 50);
    assert_eq!(client.votes_at(&c, &12), 50);
    assert_eq!(client.votes_at(&c, &10), 100);
    env.ledger().set_sequence_number(17);
    assert_eq!(client.finalize(&proposal), Outcome::Succeeded);
    assert_eq!(client.finalize(&proposal), Outcome::Succeeded);
}

#[test]
fn intra_ledger_writes_coalesce_and_snapshots_cannot_be_reused() {
    let (env, id, a, b, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &100);
    client.register(&b);
    env.ledger().set_sequence_number(11);
    let proposal = client.propose(&a, &BytesN::from_array(&env, &[2; 32]));
    assert_eq!(client.try_vote(&proposal, &a, &Choice::For), Err(Ok(Error::NotActive)));
    client.transfer(&a, &b, &70);
    client.transfer(&b, &a, &20);
    assert_eq!(client.try_votes_at(&a, &11), Err(Ok(Error::UnclosedLedger)));
    env.ledger().set_sequence_number(12);
    assert_eq!(client.balance_at(&a, &10), 100);
    assert_eq!(client.balance_at(&a, &11), 50);
    assert_eq!(client.balance_at(&b, &11), 50);
    assert_eq!(client.vote(&proposal, &a, &Choice::For), 50);
    client.transfer(&a, &b, &50);
    assert_eq!(client.vote(&proposal, &b, &Choice::Against), 50);
    assert_eq!(client.get_proposal(&proposal).votes_against, 50);
    assert_eq!(client.try_finalize(&proposal), Err(Ok(Error::NotFinished)));
    env.ledger().set_sequence_number(17);
    assert_eq!(client.finalize(&proposal), Outcome::Defeated);
    assert!(client.try_vote(&proposal, &a, &Choice::For).is_err());
}

#[test]
fn cycle_rejection_is_atomic_and_abstain_counts_for_quorum() {
    let (env, id, a, b, c) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &40);
    client.mint(&b, &10);
    client.mint(&c, &50);
    client.delegate(&a, &Some(b.clone()));
    client.delegate(&b, &Some(c.clone()));
    assert_eq!(client.try_delegate(&c, &Some(a.clone())), Err(Ok(Error::DelegationCycle)));
    assert_eq!(client.try_delegate(&c, &Some(c.clone())), Err(Ok(Error::DelegationCycle)));
    client.delegate(&b, &None);
    env.ledger().set_sequence_number(11);
    assert_eq!(client.votes_at(&c, &10), 50);
    assert_eq!(client.votes_at(&b, &10), 50);
    assert!(client.try_propose(&a, &BytesN::from_array(&env, &[0; 32])).is_err());
    let p = client.propose(&b, &BytesN::from_array(&env, &[3; 32]));
    env.ledger().set_sequence_number(12);
    client.vote(&p, &b, &Choice::Abstain);
    client.vote(&p, &c, &Choice::For);
    env.ledger().set_sequence_number(17);
    assert_eq!(client.finalize(&p), Outcome::Succeeded);
}

#[test]
fn below_quorum_fails_and_deadline_is_inclusive() {
    let (env, id, a, b, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &20);
    client.mint(&b, &80);
    env.ledger().set_sequence_number(11);
    let p = client.propose(&a, &BytesN::from_array(&env, &[4; 32]));
    env.ledger().set_sequence_number(16);
    assert_eq!(client.state(&p), Outcome::Active);
    assert_eq!(client.vote(&p, &a, &Choice::For), 20);
    env.ledger().set_sequence_number(17);
    assert_eq!(client.state(&p), Outcome::Defeated);
    assert_eq!(client.finalize(&p), Outcome::Defeated);
}

#[test]
fn mint_requires_admin_authorization() {
    let (env, id, a, _, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    env.mock_auths(&[]);
    assert!(client.try_mint(&a, &10).is_err());
}

#[test]
fn later_mint_cannot_change_quorum_or_supply_snapshot() {
    let (env, id, a, b, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &10);
    env.ledger().set_sequence_number(11);
    let p = client.propose(&a, &BytesN::from_array(&env, &[5; 32]));
    env.ledger().set_sequence_number(12);
    client.mint(&b, &1_000);
    assert_eq!(client.supply_at(&11), 10);
    assert_eq!(client.vote(&p, &a, &Choice::For), 10);
    assert!(client.try_vote(&p, &b, &Choice::For).is_err());
    env.ledger().set_sequence_number(17);
    assert_eq!(client.finalize(&p), Outcome::Succeeded);
}

#[test]
fn invalid_transfers_and_capacity_failure_do_not_mutate_state() {
    let (env, id, a, b, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &100);
    assert_eq!(client.try_transfer(&a, &b, &101), Err(Ok(Error::InsufficientBalance)));
    assert_eq!(client.try_transfer(&a, &b, &-1), Err(Ok(Error::InvalidAmount)));
    for _ in 1..MAX_ACCOUNTS {
        client.mint(&Address::generate(&env), &1);
    }
    assert_eq!(client.try_mint(&b, &10), Err(Ok(Error::Capacity)));
    env.ledger().set_sequence_number(11);
    assert_eq!(client.balance_at(&a, &10), 100);
    assert_eq!(client.supply_at(&10), 163);
}

#[test]
fn outsiders_cannot_exhaust_membership_by_delegating_or_receiving_dust() {
    let (env, id, a, b, c) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &100);
    assert_eq!(client.try_delegate(&b, &Some(c.clone())), Err(Ok(Error::UnknownMember)));
    assert_eq!(client.try_delegate(&a, &Some(b.clone())), Err(Ok(Error::UnknownMember)));
    assert_eq!(client.try_transfer(&a, &b, &1), Err(Ok(Error::UnknownMember)));
    client.register(&b);
    client.transfer(&a, &b, &1);
    client.delegate(&b, &Some(a.clone()));
    env.ledger().set_sequence_number(11);
    assert_eq!(client.votes_at(&a, &10), 100);
    assert_eq!(client.balance_at(&b, &10), 1);
    env.mock_auths(&[]);
    assert!(client.try_register(&c).is_err());
}

#[test]
fn shared_delegation_paths_and_subtree_moves_conserve_total_votes() {
    let (env, id, a, b, c) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &20);
    client.mint(&b, &30);
    client.mint(&c, &50);
    client.delegate(&a, &Some(b.clone()));
    client.delegate(&b, &Some(c.clone()));

    // Both transfer paths end at C. Only owned balances move; A's incoming
    // delegation must remain attached to B when B transfers its own tokens.
    client.transfer(&b, &a, &10);
    client.transfer(&a, &c, &5);
    env.ledger().set_sequence_number(11);
    assert_eq!(client.balance_at(&a, &10), 25);
    assert_eq!(client.balance_at(&b, &10), 20);
    assert_eq!(client.balance_at(&c, &10), 55);
    assert_eq!(client.votes_at(&c, &10), 100);

    // A moves directly beneath its existing ancestor without changing C's
    // combined weight. Removing B's delegation then releases only B's balance.
    client.delegate(&a, &Some(c.clone()));
    client.delegate(&b, &None);
    env.ledger().set_sequence_number(12);
    assert_eq!(client.votes_at(&a, &11), 0);
    assert_eq!(client.votes_at(&b, &11), 20);
    assert_eq!(client.votes_at(&c, &11), 80);
    assert_eq!(client.votes_at(&c, &10), 100);
}

#[test]
fn checkpoint_count_coalesces_and_new_mints_wait_for_a_closed_ledger() {
    let (env, id, a, b, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &100);
    client.register(&b);
    assert_eq!(client.try_propose(&a, &BytesN::from_array(&env, &[6; 32])),
        Err(Ok(Error::Threshold)));
    client.transfer(&a, &b, &70);
    client.transfer(&b, &a, &20);
    env.as_contract(&id, || {
        let count: u32 = env.storage().persistent()
            .get(&Key::Count(Series::Balance(a.clone()))).unwrap();
        assert_eq!(count, 1);
        let count: u32 = env.storage().persistent()
            .get(&Key::Count(Series::Votes(a.clone()))).unwrap();
        assert_eq!(count, 1);
    });
    env.ledger().set_sequence_number(11);
    let p = client.propose(&a, &BytesN::from_array(&env, &[6; 32]));
    assert_eq!(client.state(&p), Outcome::Pending);
    env.ledger().set_sequence_number(12);
    assert_eq!(client.state(&p), Outcome::Active);
    assert_eq!(client.vote(&p, &a, &Choice::For), 50);
}

#[test]
fn supply_overflow_rolls_back_recipient_balance_and_checkpoints() {
    let (env, id, a, b, _) = setup();
    let client = GovernanceClient::new(&env, &id);
    client.mint(&a, &i128::MAX);
    client.register(&b);
    assert_eq!(client.try_mint(&b, &1), Err(Ok(Error::Overflow)));
    env.ledger().set_sequence_number(11);
    assert_eq!(client.balance_at(&b, &10), 0);
    assert_eq!(client.votes_at(&b, &10), 0);
    assert_eq!(client.supply_at(&10), i128::MAX);
    let p = client.propose(&a, &BytesN::from_array(&env, &[7; 32]));
    env.ledger().set_sequence_number(12);
    assert_eq!(client.vote(&p, &a, &Choice::For), i128::MAX);
    env.ledger().set_sequence_number(17);
    assert_eq!(client.finalize(&p), Outcome::Succeeded);
}
