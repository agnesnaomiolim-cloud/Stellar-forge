# Parallel catchup archive downloader

`controller/src/catchup/` implements streamed HTTP Range downloads with a bounded
Tokio task pool. The root operator exports that source as `stellar_k8s::catchup`
and the existing S3 snapshot bootstrap reconciler uses it through presigned GET
URLs. This replaces that reconciler’s sequential archive body fetch. It does not
replace Stellar Core’s internal history command configuration: the repository
has no operator-owned HTTP history fetcher wired into Core. Callers downloading
history archives can use the same `Downloader` API with trusted manifest hashes.
History Archive State JSON is usually small; range splitting targets large
compressed history objects, not an assumption that HAS JSON is large.

A GET `bytes=0-0` probe discovers exact object length and optional strong ETag.
Requests specify identity encoding and reject incorrect status, Content-Range,
length, changed objects that invalidate If-Range, and excess body bytes. Servers
ignoring the initial range fall back to a single streamed GET. Unsupported or
malformed range responses fail rather than assemble ambiguous bytes. SHA-256
verification is mandatory; the unique staging file is published only after its
size and trusted digest match. A checksum must come from a trusted manifest or
archive metadata/sidecar obtained through an authenticated source. Snapshot
extraction now explicitly rejects a failed second verifier result as well.

Snapshot staging names hash the bucket and complete object key instead of
interpreting cloud keys as local paths. This prevents Windows path separators,
drive/stream syntax and reserved filenames from escaping the staging directory,
and distinguishes objects with the same basename. Temporary-file creation runs
on Tokio's blocking pool. Failed range workers abort and reap their sibling
tasks before returning; cancellation cleanup is still best effort on Windows
while a filesystem operation holds an open handle. Transparent HTTP
decompression is disabled explicitly even when another dependency enables
reqwest compression features.

Requests retry transport connection/timeouts and HTTP 5xx/429 responses with
250 ms exponential delays capped at eight seconds (four retries by default).
Interrupted chunk bodies retry from the beginning of that range; deterministic
protocol and checksum failures do not retry. Full-body fallback retries
interrupted bodies too. Header retries and body retries each have their own
bounded counters. Individual requests time out after 300 seconds; retries can
extend total archive time. Signed URLs cannot outlive the backing credentials.
Expired credentials require a new reconciliation/signature.

## Egress allocation

All snapshot reconciler instances in one operator process share a semaphore and
byte budget. Set `STELLAR_CATCHUP_WORKERS` (default 8) and
`STELLAR_CATCHUP_BYTES_PER_SECOND` (default 33554432 = 32 MiB/s) before starting
the operator. Invalid limits fail the download instead of disabling shaping.
The configuration is read once. The public API accepts an explicit shared
`Arc<WorkerPool>` so applications can allocate a different budget.

The limiter accounts for each received buffer, including retries. TCP/TLS
buffers and up to one received body chunk per active request can arrive before
pacing, so this is an application throughput budget rather than a wire-level
peak cap. It does not coordinate independent pods. Divide the worker-node
catchup egress allocation between all colocated operator replicas/downloaders,
and use node/CNI traffic shaping for a strict shared node ceiling. Reserve
capacity for active pods. Deploying every replica with the full node allowance
would exceed that allowance. Checksum sidecar and discovery requests remain SDK
metadata operations outside the body-download limiter.

## Validation status and benchmark procedure

No code, tests, builds, or benchmarks were executed for this change, following
the task instruction. The 50% speedup target remains unverified. No measured
performance data is claimed. Test source covers real local HTTP requests,
chunk reassembly, service-error retry, ignored-range fallback, incorrect ranges,
checksum rejection, interrupted probe/chunk body retry, concurrent request bounds,
capped backoff, and aggregate bandwidth accounting.
Concurrency coverage uses a barrier between two HTTP handlers rather than
timing assumptions; test-server handlers are owned and cancelled together.
Regression source also covers safe staging names, encoded responses and
incorrect declared response lengths.
Additional failure-injection coverage for cancellation remains
recommended before production rollout.

When execution is authorized, first run:

```sh
cargo test --manifest-path controller/Cargo.toml catchup
```

Create `three-month-gap.tsv` containing exactly the immutable checkpoint,
ledger, transaction, and bucket archive objects needed for one fixed three-month
ledger gap. Each line is a trusted compressed-object SHA256, a tab, and its HTTPS
GET URL. Preserve this manifest privately if URLs contain signed credentials.
Record source ledger, target ledger, checkpoint count, total compressed bytes,
provider/region, instance type, disk, CPU and memory limits, and Core version.
The example measures archive acquisition and verification; it does not replay
ledgers, so report full Core catchup time separately.

After authorization, use the same manifest and fresh output directories for
both configurations:

```sh
cargo run --release --manifest-path controller/Cargo.toml --example catchup-download -- three-month-gap.tsv sequential-output 1 33554432 18446744073709551615
cargo run --release --manifest-path controller/Cargo.toml --example catchup-download -- three-month-gap.tsv parallel-output 8 33554432 8388608
```

The sequential configuration uses one request at a time and one whole-object
range after the shared probe. It includes the same verification cost as the
parallel run. Both configurations have identical egress limits; parallelism
cannot beat a saturated bandwidth ceiling by itself. Use a fresh destination
for each repeat, alternate ordering, control proxy/CDN cache state, and perform
at least five repetitions. Also measure the pre-change SDK download path on the
same immutable snapshot to quantify integration improvement directly.

Run each pair under documented controlled network profiles, for example:
10 ms/1 Gbit/s, 100 ms/100 Mbit/s, and 200 ms/20 Mbit/s, then a profile with
injected transient 503s. Apply the same shaping and egress limit to both cases.
Record actual RTT, loss, throughput, concurrent requests, retries, CPU, peak RSS,
disk I/O, and neighbor-pod latency/throughput. Network shaping must be applied
outside the process by the test environment administrator.

Fill a results table with measured values only:

| Profile | Gap bytes | Sequential median seconds | Parallel median seconds | Reduction | Neighbor impact |
| --- | --- | --- | --- | --- | --- |
| Pending execution | — | — | — | — | — |

Compute reduction as `100 * (sequential - parallel) / sequential`. Report
variation and all failed runs. The issue’s at-least-50% reduction requires real
measurements; bandwidth-bound profiles may not achieve it. Attach raw output,
manifest identifiers without credentials, shaping configuration, and full Core
replay timing before claiming that acceptance criterion is met.
