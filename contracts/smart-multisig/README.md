# Smart Multi-Sig

`smart-multisig` is a Soroban smart contract implementing an M-of-N multi-signature wallet with a programmable on-chain `ExecutionPolicy`. Authorized signers can propose and approve outbound token transfers, which only execute once the required approval threshold is met and real-time policy guards (network base-fee ceiling and cross-contract TWAP oracle price-deviation check) pass. An admin address can update the execution policy or rotate admin rights at any time.

## Public Functions

- `__constructor(env, admin, signers, threshold, policy) -> Result<(), PolicyError>` — Initializes the wallet at deployment with the admin address, authorized signer set, approval threshold, and initial execution policy.
- `propose_transfer(env, proposer, token, to, amount) -> Result<u64, PolicyError>` — Allows an authorized signer to propose a positive outbound token transfer, automatically recording the proposer's approval and returning the new transaction ID.
- `approve(env, signer, tx_id) -> Result<(), PolicyError>` — Records an approval from an authorized signer who has not yet signed the specified pending transaction.
- `execute_transfer(env, caller, tx_id, current_fee_stroops) -> Result<(), PolicyError>` — Verifies the execution policy pre-checks and approval threshold for a pending transaction, marks it executed, and dispatches the token transfer.
- `set_policy(env, caller, new_policy) -> Result<(), PolicyError>` — Allows the admin to replace the current `ExecutionPolicy`.
- `rotate_admin(env, caller, new_admin) -> Result<(), PolicyError>` — Allows the current admin to transfer admin rights to a new address.
- `get_policy(env) -> Result<ExecutionPolicy, PolicyError>` — Returns the current `ExecutionPolicy`.
- `get_pending_tx(env, tx_id) -> Result<PendingTx, PolicyError>` — Returns the pending transaction record for the given transaction ID.
- `get_signers(env) -> Result<Vec<Address>, PolicyError>` — Returns the current list of authorized signer addresses.
- `get_threshold(env) -> Result<u32, PolicyError>` — Returns the minimum number of approvals required to execute a transfer.
- `get_admin(env) -> Result<Address, PolicyError>` — Returns the current admin address.
- `approval_count(env, tx_id) -> u32` — Returns the number of valid signer approvals collected for the given transaction ID.

## Build and Test

Run unit tests:

```bash
cargo test --manifest-path contracts/smart-multisig/Cargo.toml
```

Build the WebAssembly contract artifact:

```bash
cargo build --manifest-path contracts/smart-multisig/Cargo.toml --target wasm32-unknown-unknown --release
```
