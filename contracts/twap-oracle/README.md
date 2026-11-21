# TWAP Oracle

`twap-oracle` is a keeper-updated, single-asset time-weighted average price
oracle for Soroban. It records price observations in ledger checkpoints and
uses the elapsed ledger time to calculate TWAP values over configured windows.

An authorized updater submits positive prices, while the admin can replace that
updater. Consumers can query the configured default window or choose a window
up to the contract's configured maximum.

## Public functions

- `initialize` - Configure the admin, updater, default TWAP window, maximum window, and checkpoint capacity; callable once.
- `set_updater` - Replace the authorized updater address; callable by the admin.
- `update` - Record a positive price observation at the current ledger; requires updater authorization.
- `get_twap` - Return the TWAP over the configured default window.
- `get_twap_for_window` - Return the TWAP over a caller-selected ledger window.

## Build and test

Run the contract tests:

```sh
cargo test --manifest-path contracts/twap-oracle/Cargo.toml
```

Build the Wasm artifact:

```sh
cargo build --manifest-path contracts/twap-oracle/Cargo.toml --target wasm32-unknown-unknown --release
```
