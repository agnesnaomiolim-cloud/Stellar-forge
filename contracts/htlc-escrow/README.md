# HTLC Escrow Contract

This Soroban smart contract implements a Hash Time-Locked Contract (HTLC) escrow for token transfers between a sender and a receiver. A sender locks tokens under a 32-byte hashlock and a ledger timestamp timelock, allowing the receiver to claim the funds by providing the matching preimage before expiration. If the timelock expires without a claim, the sender can refund the locked tokens back to their account.

## Public Functions

- `deposit(env, sender, receiver, token, amount, hashlock, timelock)` — Transfers tokens from the authorized sender into the contract and stores a new escrow entry keyed by the hashlock.
- `claim(env, receiver, hashlock, preimage)` — Verifies the receiver's authorization, timelock validity, and preimage hash before marking the escrow claimed and transferring the tokens to the receiver.
- `refund(env, sender, hashlock)` — Allows the authorized sender to reclaim unclaimed tokens after the escrow's timelock has expired.

## Build and Test

Run the following commands from the `contracts/htlc-escrow` directory:

```bash
# Build the contract
cargo build --target wasm32-unknown-unknown --release

# Run tests
cargo test
```
