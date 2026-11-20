# Emergency Circuit Breaker Contract (`emergency-breaker`)

An M-of-N multi-signature emergency circuit breaker contract for Stellar/Soroban that lets a quorum of trusted operators immediately freeze critical operations. It uses Ed25519 signatures over domain-separated SHA-256 messages, enforces a timelock before operations can be unfrozen, and checks pause status in O(1) time using a `FreezeScope` bitmask.

## Public Functions

All public methods are implemented on `CircuitBreaker` in `src/lib.rs`:

- `new()` — Creates a new, uninitialized `CircuitBreaker` instance.
- `initialize(threshold, operators, timelock_delay)` — Initializes the contract with the M-of-N signature threshold, authorized operator Ed25519 public keys, and timelock duration in seconds.
- `assert_not_frozen(op)` — Verifies that the given `FreezeScope` is not frozen and returns `Err(BreakerError::OperationFrozen)` if paused.
- `is_frozen(op)` — Returns `true` if the given `FreezeScope` is currently frozen.
- `state(now)` — Returns the current `BreakerState` (`Active`, `Frozen`, or `PendingThaw`) for the provided Unix timestamp.
- `freeze(scope, sigs, now)` — Immediately freezes the specified `FreezeScope` after verifying M-of-N operator signatures and starts the timelock.
- `unfreeze(sigs, now)` — Unfreezes all operations after the timelock expires and M-of-N operator signatures are verified.

## Build and Test

Run the following commands from the `contracts/emergency-breaker` directory:

```bash
# Build the contract
cargo build

# Run the unit tests
cargo test
```
