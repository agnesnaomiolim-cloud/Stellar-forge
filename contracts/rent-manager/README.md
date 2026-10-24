# Rent Manager

This Soroban vault keeps deposited SEP-41 tokens accounted by user and extends
the contract instance and only the persistent user balance entries used by a
call. Inactive balances are not touched by other users' interactions and may be
archived normally.

## State and TTL policy

- The asset, administrator, and aggregate deposits are in instance storage.
- Each user's balance is a separate persistent key.
- Public calls refresh the instance TTL. Balance reads/writes refresh only the
  participating user's key, and absent keys are not extended.
- TTL targets are fixed ledger lifetimes (30 days at 17,280 ledgers/day).
  They remain below Soroban's maximum TTL. No rent amount is estimated or
  accepted from callers: the protocol computes the multidimensional rent
  resource fee from the actual ledger entries and requested extension.

## Restoring archived state

A contract cannot restore an archived ledger entry from inside its Wasm
execution. When a balance (or the contract instance/WASM) has been archived,
the user must first submit a transaction containing the protocol's
`RestoreFootprint` operation for the archived entries. That restoration
transaction pays the rent resource fee from its transaction fee source. After
it succeeds, call `restore(owner)` to extend that balance's TTL and read the
unchanged stored amount. The refresh call is permissionless so a keeper can
submit it and pay its transaction fee for the user.

If the contract instance or code is archived too, include the required
instance/code ledger entries in the restoration footprint as well. A failed or
underfunded restoration transaction does not change contract state. The rent
fee is protocol-calculated; this contract does not claim that a fixed
user-supplied amount is sufficient.

This flow assumes the archived entry is still retained by the network and
eligible for restoration. TTL management cannot prevent eventual pruning of
data that remains archived beyond the protocol's archival retention period.

## Tests

Run `cargo test --manifest-path contracts/rent-manager/Cargo.toml`. The
10,000-ledger test checks that a restored balance remains intact across a
long ledger jump and that the proxy refreshes the balance. It is not a local
network archival integration test: Soroban unit-test invocation does not
construct and submit a protocol `RestoreFootprint` transaction. Verify the
full archived-entry flow on a local network by archiving a test balance,
restoring its footprint with the network transaction fee estimator, then
invoking `restore` and checking `balance` and `total_deposits`.
