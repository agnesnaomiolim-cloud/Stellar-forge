# Escrow Contract

A decentralized on-chain Soroban escrow contract with multi-party arbitration and time-locked refunds. A buyer locks tokens into an escrow for a seller and a designated arbiter, with a 1% protocol fee applied upon completion or dispute resolution. Unconfirmed escrows can be refunded in full to the buyer once the ledger-based timelock deadline expires.

## Public Functions

- `initialize(env, admin, protocol_fee_address)` — Initializes the contract with an admin address and a protocol fee recipient address.
- `fund_escrow(env, buyer, seller, arbiter, token, amount, timelock_ledgers) -> u32` — Transfers tokens from the buyer into the contract and creates a new escrow in the `Funded` state, returning its ID.
- `confirm_deal(env, id, seller)` — Allows the seller to confirm a funded escrow, transitioning it from `Funded` to `Active`.
- `raise_dispute(env, id, caller)` — Allows the buyer or seller to raise a dispute on an active escrow, transitioning it from `Active` to `Disputed`.
- `resolve_dispute(env, id, arbiter, outcome)` — Allows the designated arbiter to resolve a disputed escrow, transferring the protocol fee and splitting the remaining funds according to the outcome (`Resolved`).
- `refund_expired(env, id)` — Allows anyone to refund the full escrowed amount (with no fee) to the buyer after the deadline has passed on an unconfirmed `Funded` escrow (`Refunded`).
- `release_funds(env, id, seller)` — Allows the seller to complete an active escrow, paying the protocol fee and releasing the remaining funds to the seller (`Completed`).
- `get_escrow(env, id) -> Escrow` — Returns the `Escrow` record for the given escrow ID.
- `get_admin(env) -> Address` — Returns the contract's admin address.
- `get_protocol_fee_address(env) -> Address` — Returns the protocol fee recipient address.

## Build and Test

Run the following commands from `contracts/escrow`:

```bash
# Build the contract
cargo build --target wasm32-unknown-unknown --release

# Run unit tests
cargo test
```
