# Governance delegation and voting registry

`contracts/gov-registry` is an independent Soroban SDK 27.0.6 workspace. It is a
governance-only ledger, not an SEP-41 token wrapper. Its deployment constructor
fixes the mint authority, proposal threshold, quorum basis points, and voting
duration. Minting requires admin authorization; transfers, delegation, proposals,
and voting require the acting address's authorization. There is no arbitrary
proposal execution or privileged tally override.

## Delegation and historical values

Every member initially votes for itself. `delegate(A, Some(B))` transfers A's
entire subtree voting weight to B's delegation chain. A → B → C makes C carry
all three balances; A and B have zero voting power. `delegate(B, None)` reclaims
B's and A's weight. Transfers move only the sender's own tokens, updating both
delegation paths. Cycles and self-delegation are rejected before writes.

Membership is capped at 64 distinct accounts for this initial registry. The admin
admits accounts through `register` or `mint`; transfers and delegation accept only
admitted recipients. Unfunded outsiders cannot reserve slots through delegation,
and token holders cannot fill the registry by sending dust to arbitrary addresses.
Membership admission is therefore permissioned, even though members control their
own delegation and transfers. Accounts
cannot be removed or slots recycled. This bounds every graph walk to 64 entries,
including when a subtree moves beneath a longer chain. This is an explicit small
committee deployment limit; larger DAOs need a different graph representation.
Historical points use individual persistent keys and binary search, so looking
up one ledger does not load an unbounded vector. Multiple writes in one ledger
replace its last checkpoint.

## Snapshot and lifecycle semantics

Proposal eligibility uses voting power at the previous **closed** ledger, avoiding
same-ledger mint/borrow eligibility. The snapshot is the **end of the proposal
creation ledger**. Transactions later in that ledger are included; exact
transaction-order snapshots are not exposed by this API. Voting starts in the
following ledger and ends at the inclusive `snapshot + voting_ledgers` deadline.
Current and future historical queries are rejected. This closed-ledger rule
prevents intra-ledger transfers from recycling power after a vote.

One persistent receipt per `(proposal, voter)` prevents duplicate votes. Subsequent
transfers or redelegation cannot affect the creation-ledger voting checkpoint.
For, Against, and Abstain count toward quorum, rounded up from total snapshot
supply. Success additionally requires For > Against; ties and abstain-only votes
fail. Finalization is permissionless, idempotent, and allowed only after voting.
`get_proposal` returns the description hash, snapshot, deadline, and all tallies;
the proposal body is stored off-chain by its hash.

## Storage lifetime

Written persistent entries extend to the network maximum TTL. Voting duration must
be shorter than that TTL, keeping receipts live through the entire voting window.
The instance is refreshed on configuration access. Historical checkpoint data is
never pruned. Old archival entries must be restored through Soroban transaction
simulation/restoration before use; unavailable history fails the transaction rather
than replacing a known account with a new balance. Operators must fund archival
restoration and instance/code TTL maintenance. See the SDK's [persistent storage
API](https://docs.rs/soroban-sdk/27.0.6/soroban_sdk/storage/struct.Persistent.html)
and Stellar's [TTL guide](https://developers.stellar.org/docs/build/guides/conventions/extending-wasm-ttl).

## Validation (not executed)

Tests in `src/test.rs` cover chain aggregation, subtree redelegation, cycle rejection,
authorization, coalesced intra-ledger transfers, immutable snapshots, duplicate
votes, threshold, quorum, abstention, ties, and lifecycle boundaries. Run later:

```sh
cargo test --manifest-path contracts/gov-registry/Cargo.toml
cargo build --manifest-path contracts/gov-registry/Cargo.toml --release --target wasm32v1-none
```

No tests, builds, deployment, or security audit were executed for this change,
as requested. This contract requires those checks and an independent audit before
deployment with economically meaningful voting power.
