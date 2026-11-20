# Dutch Auction Contract

A deterministic, front-running-resistant token distribution contract for Stellar/Soroban. The auction starts at a configured `start_price` and linearly decays to `reserve_price` over time, with all participants paying the same final clearing price locked in when the auction sells out or reaches `end_time`. After settlement, bidders claim their allocated project tokens along with any unused reserve-token refund.

## Public Functions

- `initialize(env, admin, token, reserve_token, total_tokens, start_price, reserve_price, start_time, end_time)` — Configures the auction parameters once and sets the initial status to `Pending`.
- `start(env, caller)` — Transitions the auction from `Pending` to `Open` (admin only, callable at or after `start_time`).
- `commit(env, bidder, amount)` — Deposits reserve tokens into an open auction to reserve project tokens, automatically settling if the remaining supply sells out.
- `settle(env, caller)` — Locks in the final clearing price and transitions the auction to `Settled` once sold out or past `end_time` (admin only).
- `claim(env, bidder)` — Distributes purchased project tokens and any unused reserve-token refund to a bidder after settlement (callable once per user).
- `price(env)` — Returns the current price per token based on the current ledger timestamp.
- `status(env)` — Returns the current auction lifecycle status (`Pending`, `Open`, or `Settled`).
- `clearing_price(env)` — Returns the locked-in clearing price after the auction has settled.
- `user_deposit(env, user)` — Returns the total amount of reserve tokens deposited by `user`.
- `user_claimed(env, user)` — Returns whether `user` has already claimed their tokens and refund.
- `tokens_remaining(env)` — Returns the number of project tokens not yet committed.

## Build & Test

Run these commands from the `contracts/dutch-auction` directory:

```bash
# Build the contract to WebAssembly
cargo build --target wasm32-unknown-unknown --release

# Run unit tests
cargo test
```
