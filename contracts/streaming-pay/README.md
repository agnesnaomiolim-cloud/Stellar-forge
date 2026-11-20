# Streaming Pay

`streaming-pay` is a Soroban smart contract that enables linear token payment streams between a payer and a payee over a fixed duration. A payer locks funds in the contract when creating a stream, and tokens unlock linearly over time based on the ledger timestamp. The payee can withdraw unlocked tokens at any point, and the payer can cancel the stream early to settle accrued funds and reclaim the unvested remainder.

## Public Functions

- `create_stream(env, payer, payee, token, amount, duration) -> u64` — Locks a positive token amount from the authorized payer into a new linear stream for the payee over the specified duration and returns the assigned stream ID.
- `withdraw(env, stream_id)` — Transfers any linearly unlocked tokens that have not yet been withdrawn from the specified stream to the authorized payee.
- `cancel_stream(env, stream_id)` — Allows the authorized payer to cancel a stream, paying out any accrued unlocked balance to the payee, refunding the remaining locked balance to the payer, and removing the stream from storage.

## Build and Test

Run unit tests:

```bash
cargo test --manifest-path contracts/streaming-pay/Cargo.toml
```

Build the WebAssembly contract artifact:

```bash
cargo build --manifest-path contracts/streaming-pay/Cargo.toml --target wasm32-unknown-unknown --release
```
