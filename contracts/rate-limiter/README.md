# On-chain token-bucket rate limiter

Soroban contract that caps how often a **caller address** may invoke a gated
function. Refill is derived from **ledger sequence** (deterministic, no wall clock).

## Config

| Field | Meaning |
|---|---|
| `capacity` | Burst size (tokens). Must be > 0. |
| `refill_rate` | Tokens added per ledger sequence. |

State: one `TokenBucket { tokens, last_refill }` persistent entry per `Address`.

## Apply to an entry point

```rust
use rate_limiter::{enforce, rate_limited, RateLimitError};

pub fn mint(env: Env, caller: Address) -> Result<(), RateLimitError> {
    caller.require_auth();
    rate_limited!(env, caller, {
        // mint logic
        Ok(())
    })
}
```

`enforce(&env, &caller)?` is the non-macro form. Exhausted buckets return
`RateLimitError::RateLimited` (no panic).

## Arithmetic

`tokens = min(capacity, tokens + saturating_mul(elapsed_ledgers, refill_rate))`
then subtract the request. Independent buckets cannot be drained by another address.
