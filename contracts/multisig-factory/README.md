# Multisig Factory

`multisig-factory` is a Soroban smart contract that deploys and tracks independent multi-signature wallet instances for DAO treasuries and institutional custodians. An administrator registers a compiled wallet WASM hash, and callers can then deploy unique wallet contracts seeded with their own signer sets and approval thresholds using a 32-byte salt. The factory acts as a canonical registry of deployed wallet addresses while holding no signing authority over the wallets it creates.

## Public Functions

- `initialize(env, admin)` — Initializes the factory once with the administrator address and sets the deployed wallet counter to zero.
- `register_wallet_wasm(env, wasm_hash)` — Registers or updates the uploaded wallet WASM hash used for future deployments (admin only).
- `deploy_wallet(env, salt, signers, threshold)` — Deploys a new multi-signature wallet instance for a unique salt, signer list, and approval threshold, and returns its address.
- `get_wallet(env, salt)` — Returns the address of a previously deployed wallet for the given salt, if one exists.
- `wallet_count(env)` — Returns the total number of wallets deployed through this factory.
- `admin(env)` — Returns the current factory administrator address.
- `wallet_wasm_hash(env)` — Returns the currently registered wallet WASM hash, if any.
- `rotate_admin(env, new_admin)` — Transfers the factory administrator role to a new address (admin only).

## Build and Test

This crate is a standalone Soroban workspace. Run the following commands from `contracts/multisig-factory`:

```bash
# Build the contract for Soroban (WASM)
cargo build --target wasm32-unknown-unknown --release

# Run unit tests
cargo test
```
