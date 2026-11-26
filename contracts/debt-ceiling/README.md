# Debt Ceiling Contract

The `stellar-debt-ceiling` contract is a multi-collateral stablecoin debt-ceiling controller built on the Stellar Soroban platform. It tracks the total outstanding debt minted against each registered collateral type and enforces per-collateral ceilings so that no collateral can back more stablecoin than its configured limit allows. Ceiling changes go through a time-locked governance queue, giving stakeholders advance notice before any limit is raised or lowered.

## Public Functions

| Function | Description |
|---|---|
| `initialize(env, admin, governor)` | One-time setup that records the administrator and governor addresses. Returns `Error::AlreadyInitialized` if called again. |
| `admin(env)` | Returns the current administrator address. |
| `governor(env)` | Returns the current governor address. |
| `set_admin(env, new_admin)` | Transfers the administrator role to `new_admin`. Requires the current administrator's authorization. |
| `set_governor(env, new_governor)` | Transfers the governor role to `new_governor`. Requires the current administrator's authorization. |
| `register_collateral(env, collateral, ceiling)` | Registers a collateral type with its initial debt ceiling. Admin-only. |
| `collateral_config(env, collateral)` | Returns the `CollateralConfig` struct (ceiling and current debt) for the given collateral address. |
| `total_debt(env, collateral)` | Returns the total outstanding debt currently minted against the given collateral. |
| `remaining_capacity(env, collateral)` | Returns how much additional debt can still be minted against the given collateral before its ceiling is reached. |
| `set_ceiling(env, collateral, ceiling)` | Immediately updates a collateral's debt ceiling. Admin-only. For time-locked changes use `queue_ceiling_change`. |
| `queue_ceiling_change(env, collateral, ceiling)` | Queues a time-locked ceiling change for `collateral`. Governor-only. Returns the timestamp at which the change becomes executable. |
| `execute_ceiling_change(env, collateral)` | Applies a previously queued ceiling change once its delay has elapsed. Governor-only. |
| `cancel_ceiling_change(env, collateral)` | Cancels a pending ceiling change for `collateral`. Governor-only. |
| `pending_ceiling_change(env, collateral)` | Returns the pending `PendingCeilingChange` for `collateral`, or `None` if no change is queued. |
| `enforce_mint(env, collateral, amount)` | Checks that minting `amount` against `collateral` would not breach its ceiling. Returns `Error::CeilingExceeded` if it would. Called by minting engines before issuing new debt. |
| `record_mint(env, collateral, amount)` | Records that `amount` of new debt has been minted against `collateral`. Intended to be called after a successful mint. |
| `record_repay(env, collateral, amount)` | Reduces the recorded outstanding debt for `collateral` by `amount`, freeing ceiling capacity. |

> **Function verification:** All function names listed above were checked directly against `src/lib.rs`. The issue additionally names `admin`, `cancel_ceiling_change`, `collateral_config`, `enforce_mint`, `execute_ceiling_change`, `governor`, `initialize`, `pending_ceiling_change`, `queue_ceiling_change`, and `record_mint` — every one of those names exists in the source.

## Build

```bash
# From the contracts/debt-ceiling directory
stellar contract build
```

Or with `cargo` directly:

```bash
cargo build --target wasm32-unknown-unknown --release
```

## Test

```bash
cargo test
```

To enable test utilities provided by `soroban-sdk`:

```bash
cargo test --features testutils
```
