//! Governance-only token ledger with transitive delegation and immutable voting snapshots.
#![no_std]

pub mod checkpoints;
#[cfg(test)]
mod test;

use checkpoints::Series;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Vec};

// A bounded membership set also bounds every delegation traversal, including subtree moves.
const MAX_ACCOUNTS: u32 = 64;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    InvalidConfig = 1, InvalidAmount = 2, InsufficientBalance = 3,
    Overflow = 4, Capacity = 5, DelegationCycle = 6, UnclosedLedger = 7,
    MissingHistory = 8, Threshold = 9, UnknownProposal = 10,
    NotActive = 11, AlreadyVoted = 12, NoVotingPower = 13, NotFinished = 14,
}

#[contracttype]
#[derive(Clone)]
pub(crate) enum Key {
    Config, Members, Account(Address), Count(Series), Point(Series, u32),
    NextProposal, Proposal(u32), Receipt(u32, Address),
}

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub admin: Address,
    pub proposal_threshold: i128,
    pub quorum_bps: u32,
    pub voting_ledgers: u32,
}

#[contracttype]
#[derive(Clone)]
struct Account {
    balance: i128,
    // Own balance plus all incoming delegated balances.
    weight: i128,
    delegate: Option<Address>,
}

/// Choices include abstention toward quorum, but only For/Against determine approval.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Choice { For, Against, Abstain }

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome { Pending, Active, Succeeded, Defeated }

#[contracttype]
#[derive(Clone)]
pub struct Proposal {
    pub proposer: Address,
    pub description_hash: BytesN<32>,
    pub snapshot: u32,
    pub deadline: u32,
    pub votes_for: i128,
    pub votes_against: i128,
    pub abstain: i128,
    pub finalized: bool,
    pub passed: bool,
}

fn config(env: &Env) -> Config {
    let ttl = env.storage().max_ttl();
    env.storage().instance().extend_ttl(ttl / 2, ttl);
    env.storage().instance().get(&Key::Config).unwrap()
}

pub(crate) fn retain(env: &Env, key: &Key) {
    let ttl = env.storage().max_ttl();
    env.storage().persistent().extend_ttl(key, ttl / 2, ttl);
}

fn members(env: &Env) -> Vec<Address> {
    env.storage().instance().get(&Key::Members).unwrap_or(Vec::new(env))
}

fn account(env: &Env, who: &Address) -> Result<Account, Error> {
    if !members(env).contains(who) {
        return Ok(Account { balance: 0, weight: 0, delegate: None });
    }
    env.storage().persistent().get(&Key::Account(who.clone())).ok_or(Error::MissingHistory)
}

fn save(env: &Env, who: &Address, value: &Account) -> Result<(), Error> {
    let mut list = members(env);
    if !list.contains(who) {
        if list.len() >= MAX_ACCOUNTS { return Err(Error::Capacity); }
        list.push_back(who.clone());
        env.storage().instance().set(&Key::Members, &list);
    }
    env.storage().persistent().set(&Key::Account(who.clone()), value);
    retain(env, &Key::Account(who.clone()));
    checkpoints::write(env, Series::Balance(who.clone()), value.balance)?;
    checkpoints::write(env, Series::Votes(who.clone()),
        if value.delegate.is_none() { value.weight } else { 0 })
}

fn propagate(env: &Env, start: &Address, delta: i128) -> Result<(), Error> {
    let mut next = Some(start.clone());
    let mut visited = Vec::new(env);
    while let Some(who) = next {
        if visited.contains(&who) || visited.len() >= MAX_ACCOUNTS { return Err(Error::DelegationCycle); }
        visited.push_back(who.clone());
        let mut value = account(env, &who)?;
        value.weight = value.weight.checked_add(delta).ok_or(Error::Overflow)?;
        if value.weight < 0 { return Err(Error::InsufficientBalance); }
        next = value.delegate.clone();
        save(env, &who, &value)?;
    }
    Ok(())
}

fn proposal(env: &Env, id: u32) -> Result<Proposal, Error> {
    env.storage().persistent().get(&Key::Proposal(id)).ok_or(Error::UnknownProposal)
}

fn approved(env: &Env, p: &Proposal) -> Result<bool, Error> {
    let supply = checkpoints::lookup(env, Series::Supply, p.snapshot)?;
    let bps = i128::from(config(env).quorum_bps);
    // ceil(supply * bps / 10_000) without overflowing supply * bps.
    let quorum = (supply / 10_000) * bps + ((supply % 10_000) * bps + 9_999) / 10_000;
    let participation = p.votes_for.checked_add(p.votes_against)
        .and_then(|n| n.checked_add(p.abstain)).ok_or(Error::Overflow)?;
    Ok(participation >= quorum && p.votes_for > p.votes_against)
}

#[contract]
pub struct Governance;

#[contractimpl]
impl Governance {
    /// Deployment binds the mint authority and immutable governance parameters.
    pub fn __constructor(env: Env, settings: Config) -> Result<(), Error> {
        if settings.proposal_threshold <= 0 || settings.quorum_bps == 0
            || settings.quorum_bps > 10_000 || settings.voting_ledgers == 0
            || settings.voting_ledgers >= env.storage().max_ttl() {
            return Err(Error::InvalidConfig);
        }
        env.storage().instance().set(&Key::Config, &settings);
        env.storage().instance().set(&Key::NextProposal, &0_u32);
        checkpoints::write(&env, Series::Supply, 0)
    }

    /// Admin-controlled issuance for the governance ledger; no external token claims.
    pub fn mint(env: Env, to: Address, amount: i128) -> Result<(), Error> {
        config(&env).admin.require_auth();
        if amount <= 0 { return Err(Error::InvalidAmount); }
        let mut value = account(&env, &to)?;
        value.balance = value.balance.checked_add(amount).ok_or(Error::Overflow)?;
        save(&env, &to, &value)?;
        propagate(&env, &to, amount)?;
        let supply: i128 = env.storage().instance().get(&Series::Supply).unwrap_or(0);
        let supply = supply.checked_add(amount).ok_or(Error::Overflow)?;
        env.storage().instance().set(&Series::Supply, &supply);
        checkpoints::write(&env, Series::Supply, supply)
    }

    /// Transfer owned tokens; incoming delegated weight is never transferable.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();
        if amount <= 0 { return Err(Error::InvalidAmount); }
        let mut sender = account(&env, &from)?;
        if sender.balance < amount { return Err(Error::InsufficientBalance); }
        if from == to { return Ok(()); }
        sender.balance -= amount;
        save(&env, &from, &sender)?;
        propagate(&env, &from, -amount)?;
        let mut recipient = account(&env, &to)?;
        recipient.balance = recipient.balance.checked_add(amount).ok_or(Error::Overflow)?;
        save(&env, &to, &recipient)?;
        propagate(&env, &to, amount)
    }

    /// Move the caller's entire incoming delegation subtree; None reclaims voting power.
    pub fn delegate(env: Env, from: Address, to: Option<Address>) -> Result<(), Error> {
        from.require_auth();
        let mut value = account(&env, &from)?;
        let mut next = to.clone();
        let mut seen = Vec::new(&env);
        while let Some(who) = next {
            if who == from || seen.contains(&who) || seen.len() >= MAX_ACCOUNTS {
                return Err(Error::DelegationCycle);
            }
            seen.push_back(who.clone());
            next = account(&env, &who)?.delegate;
        }
        if value.delegate == to { return Ok(()); }
        if let Some(old) = value.delegate.clone() { propagate(&env, &old, -value.weight)?; }
        value.delegate = to.clone();
        save(&env, &from, &value)?;
        if let Some(new) = to { propagate(&env, &new, value.weight)?; }
        Ok(())
    }

    /// Historical balance at the end of a closed ledger.
    pub fn balance_at(env: Env, owner: Address, ledger: u32) -> Result<i128, Error> {
        checkpoints::lookup(&env, Series::Balance(owner), ledger)
    }

    /// Historical voting power, zero for accounts delegating at that ledger.
    pub fn votes_at(env: Env, owner: Address, ledger: u32) -> Result<i128, Error> {
        checkpoints::lookup(&env, Series::Votes(owner), ledger)
    }

    /// Historical minted supply used as the quorum denominator.
    pub fn supply_at(env: Env, ledger: u32) -> Result<i128, Error> {
        checkpoints::lookup(&env, Series::Supply, ledger)
    }

    /// Eligibility uses the previous closed ledger; voting snapshots the creation ledger.
    pub fn propose(env: Env, proposer: Address, description_hash: BytesN<32>) -> Result<u32, Error> {
        proposer.require_auth();
        let settings = config(&env);
        let now = env.ledger().sequence();
        let previous = now.checked_sub(1).ok_or(Error::UnclosedLedger)?;
        if checkpoints::lookup(&env, Series::Votes(proposer.clone()), previous)? < settings.proposal_threshold {
            return Err(Error::Threshold);
        }
        let id: u32 = env.storage().instance().get(&Key::NextProposal).unwrap();
        let next = id.checked_add(1).ok_or(Error::Overflow)?;
        let deadline = now.checked_add(settings.voting_ledgers).ok_or(Error::Overflow)?;
        let p = Proposal { proposer, description_hash, snapshot: now, deadline,
            votes_for: 0, votes_against: 0, abstain: 0, finalized: false, passed: false };
        env.storage().persistent().set(&Key::Proposal(id), &p);
        retain(&env, &Key::Proposal(id));
        env.storage().instance().set(&Key::NextProposal, &next);
        Ok(id)
    }

    /// Record one authenticated vote using end-of-creation-ledger delegated power.
    pub fn vote(env: Env, id: u32, voter: Address, choice: Choice) -> Result<i128, Error> {
        voter.require_auth();
        let mut p = proposal(&env, id)?;
        let now = env.ledger().sequence();
        if now <= p.snapshot || now > p.deadline || p.finalized { return Err(Error::NotActive); }
        let receipt = Key::Receipt(id, voter.clone());
        if env.storage().persistent().has(&receipt) { return Err(Error::AlreadyVoted); }
        let weight = checkpoints::lookup(&env, Series::Votes(voter), p.snapshot)?;
        if weight == 0 { return Err(Error::NoVotingPower); }
        let tally = match choice { Choice::For => &mut p.votes_for,
            Choice::Against => &mut p.votes_against, Choice::Abstain => &mut p.abstain };
        *tally = tally.checked_add(weight).ok_or(Error::Overflow)?;
        env.storage().persistent().set(&receipt, &true);
        retain(&env, &receipt);
        env.storage().persistent().set(&Key::Proposal(id), &p);
        retain(&env, &Key::Proposal(id));
        Ok(weight)
    }

    /// Permissionless finalization after the inclusive voting deadline.
    pub fn finalize(env: Env, id: u32) -> Result<Outcome, Error> {
        let mut p = proposal(&env, id)?;
        if env.ledger().sequence() <= p.deadline { return Err(Error::NotFinished); }
        if !p.finalized {
            p.passed = approved(&env, &p)?;
            p.finalized = true;
            env.storage().persistent().set(&Key::Proposal(id), &p);
        }
        Ok(if p.passed { Outcome::Succeeded } else { Outcome::Defeated })
    }

    /// Read proposal metadata and tallies.
    pub fn get_proposal(env: Env, id: u32) -> Result<Proposal, Error> { proposal(&env, id) }

    /// Compute lifecycle state without requiring a finalize transaction.
    pub fn state(env: Env, id: u32) -> Result<Outcome, Error> {
        let p = proposal(&env, id)?;
        let now = env.ledger().sequence();
        if now <= p.snapshot { return Ok(Outcome::Pending); }
        if now <= p.deadline { return Ok(Outcome::Active); }
        let passed = if p.finalized { p.passed } else { approved(&env, &p)? };
        Ok(if passed { Outcome::Succeeded } else { Outcome::Defeated })
    }
}
