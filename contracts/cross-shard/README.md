# Cross-Shard Liquidity State Verifier

The Cross-Shard Liquidity State Verifier is a Soroban coordinator for executing an ordered swap across multiple AMM pools in one transaction. It applies slippage and deadline checks and relies on Soroban transaction atomicity so that a failed leg reverts the entire swap; it also provides admin controls, in-flight protection, swap snapshots, and advisory pool locks.

## Public functions

- `initialize(admin)` — Set the coordinator's admin address once after deployment.
- `execute_atomic_swap(initiator, transitions)` — Execute 2–16 ordered pool transitions atomically and return each leg's output amount.
- `set_paused(caller, paused)` — Pause or unpause swaps; only the admin may call this.
- `transfer_admin(caller, new_admin)` — Transfer admin control to a new address; only the current admin may call this.
- `force_release_lock(pool)` — Release an expired advisory lock for a pool.
- `get_admin()` — Return the coordinator's admin address.
- `is_paused()` — Return whether the coordinator is paused.
- `is_inflight()` — Return whether a swap is currently being executed.
- `last_swap_id()` — Return the most recently issued swap ID.
- `get_swap_snapshot(swap_id)` — Return the stored snapshot for a swap ID, if present.
- `get_pool_lock(pool)` — Return the lock record for a pool, if present.
- `pool_is_locked(pool)` — Return whether a pool has a non-expired lock.

## Build and test

Run these commands from this directory. The WASM build requires the Rust `wasm32v1-none` target.

```sh
rustup target add wasm32v1-none
cargo build --target wasm32v1-none --release
cargo test
```
