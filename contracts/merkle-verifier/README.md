# Merkle Verifier

A Soroban-native Merkle Tree state-proof verification library written in pure Rust for on-chain proof validation. It supports iterative, stack-safe single-leaf and multi-leaf inclusion proofs using `no_std`-compatible SHA-256 hashing. Its CPU cost scales logarithmically with tree depth, keeping verification within Soroban transaction instruction budgets.

## Public Functions

- `hash_leaf(data: &[u8]) -> Hash` — Hashes a raw byte slice with SHA-256 to produce a 32-byte Merkle leaf digest.
- `verify_proof(proof: &MerkleProof, expected_root: &Hash) -> bool` — Verifies that a single leaf belongs to a Merkle tree by iteratively reconstructing the root along its proof path and comparing it to `expected_root`.
- `verify_multi_proof(proof: &MultiProof, expected_root: &Hash) -> bool` — Verifies multiple leaves simultaneously in a single bottom-up pass against `expected_root`, returning `false` for invalid tree sizes, empty leaf lists, out-of-bounds indices, or mismatched roots.

## Build and Test

Run the following commands from `contracts/merkle-verifier`:

```bash
# Build the contract library
cargo build

# Run unit tests
cargo test
```
