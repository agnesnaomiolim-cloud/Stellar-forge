# DID Registry (`did-registry`)

`did-registry` is a Soroban smart contract for managing W3C-compliant Decentralized Identifiers (DIDs) and privacy-preserving Verifiable Credentials. It maps Stellar/Soroban addresses to DID documents, lets registry-authorized issuers anchor credential hashes under semantic topics, and lets third parties verify claims without exposing personal data on-chain. DID controllers and issuers can revoke credentials, and DIDs can be permanently deactivated.

## Public Functions

All public entry points are exposed by `DidRegistry` in `src/lib.rs`:

| Function | Description |
|---|---|
| `__constructor` | Initializes the registry with an admin address and zeroed DID and credential counters. |
| `admin` | Returns the current registry admin address. |
| `set_admin` | Replaces the registry admin; requires authorization from the current admin. |
| `register_issuer` | Registers a new authorized issuer with a display name; requires admin authorization. |
| `set_issuer_status` | Enables or suspends an issuer; requires admin authorization. |
| `get_issuer` | Returns the stored `IssuerInfo` for an issuer, or `None` if it is not registered. |
| `register_did` | Creates a DID record with verification methods, relationships, and service endpoints; requires subject authorization. |
| `update_did` | Replaces an active DID's verification methods, relationships, and services; requires controller authorization. |
| `add_verification_method` | Appends a verification method to an active DID; requires controller authorization. |
| `add_service` | Appends a service endpoint to an active DID; requires controller authorization. |
| `deactivate_did` | Permanently deactivates a DID; requires controller authorization. |
| `resolve_did` | Resolves a subject's stored record into a W3C-compliant `DidDocument`. |
| `issue_credential` | Anchors a credential hash and claim commitment for a subject under a topic; callable only by an active registered issuer. |
| `revoke_credential` | Revokes a credential; callable by the subject, the original issuer, or the registry admin. |
| `get_credential` | Returns a subject's credential by ID, or `None` if it does not exist. |
| `get_credentials_by_topic` | Returns the credential IDs recorded for a subject under a given topic. |
| `verify_credential_by_id` | Verifies a credential's existence, optional expected hash, revocation state, expiry, and issuer status. |
| `verify_claim_by_topic` | Returns the first valid, unrevoked, unexpired credential for a topic, optionally restricted to a trusted issuer. |
| `get_statistics` | Returns the total number of registered DIDs and issued credentials as `(did_count, credential_count)`. |

## Build & Test Commands

Run the commands from the repository root:

```bash
# Run the contract's unit tests
cargo test --manifest-path contracts/did-registry/Cargo.toml

# Compile the contract to a release WASM binary
cargo build --manifest-path contracts/did-registry/Cargo.toml --target wasm32-unknown-unknown --release
```
