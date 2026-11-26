# Bonding Curve Contract

The `bonding-curve` contract implements a continuous token market using a Bancor-style bonding curve on the Stellar Soroban platform. It maintains a reserve of one token to back the supply of a bonded token, pricing every buy and sell algorithmically according to the configured reserve ratio and fee. Because the price moves along the curve, liquidity is always available without an order book.

## Public Functions

| Function | Description |
|---|---|
| `initialize(env, token, reserve_token, reserve_ratio, fee_percentage)` | One-time setup that records the bonded-token address, the reserve-token address, the reserve ratio (expressed in basis points out of 10 000, e.g. `5000` = 50 %), and the fee in basis points. Reverts if already initialized. |
| `buy(env, buyer, amount, max_cost)` | Mints `amount` of the bonded token for `buyer`. Calculates the reserve cost via the curve, adds the fee, checks the total against `max_cost` (slippage guard), transfers reserve tokens in, and mints bonded tokens out. |
| `sell(env, seller, amount, min_return)` | Burns `amount` of the bonded token held by `seller`. Calculates the reserve return via the curve, deducts the fee, checks the net return against `min_return` (slippage guard), burns the bonded tokens, and transfers reserve tokens out. |

> **Function verification:** All three function names above (`initialize`, `buy`, `sell`) were checked directly against `src/lib.rs` and exist in the source.

## Build

```bash
# From the contracts/bonding-curve directory
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
