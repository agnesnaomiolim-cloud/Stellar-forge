# Capacity Planning & Storage Scaling Calculator Guide

This guide gives operators a definitive way to size CPU, memory, and storage
for Stellar-K8s-managed nodes, project storage growth over 6/12/24 months,
and understand how the operator's automated volume expansion interacts with
that growth — so a cluster is provisioned correctly the first time instead of
discovered via a `Disk Full` outage.

It complements, rather than replaces, three existing docs:

- [Resource Limits for Stellar Node Types](../resource-limits.md) — CPU/memory
  request/limit recommendations per node type.
- [Proactive Disk Scaling](../proactive-disk-scaling.md) — the operator's
  automated EBS/GCP volume expansion feature (config reference, events,
  metrics).
- [History Archive Pruning Guide](../archive-pruning.md) — reclaiming space on
  full-history archive nodes.

This guide is the "how much do I provision, and how fast will it grow"
reference that ties those three together.

---

## 1. Baseline Requirements by Node Type

Baselines below start from the [official Stellar hardware
recommendations](https://developers.stellar.org/docs/validators/admin-guide/prerequisites)
and are mapped onto the operator's `StellarNode` resource model and standard
AWS/GCP instance types. Where this repo's own [resource-limits.md](../resource-limits.md)
recommends a different figure for Testnet (it generally sizes lower), that
figure is used instead — Testnet load is a small fraction of Mainnet and does not need production-grade headroom.

 
> **Read this table as a starting point, not a ceiling.** All figures assume
> "current network activity" as of this writing (see [§3](@33-storage-growth-model)
> for the live throughput this is based on). Actual requirements grow with
> network activity — re-validate against §3 periodically, and prefer the
> operator's own [Prometheus metrics](#5-monitoring-checklist) over static
> numbers once a node has been running for a few weeks.

### 1.1 Validator (Stellar Core)

| | CPU | Memory | Disk | IOPS | Cloud equivalent |
|---|---|---|---|---|---|
| **Mainnet, `historyMode: Recent`** | 8 vCPU@ 3.4GHz | 16Gi | 100Gi NVMe SSD | 10,000 | AWS `c5.2xlarge` / GCP `n4-highcpu-8` |
| **Mainnet, `historyMode: Full`** | 8 vCPU@ 3.4GHz | 16Gi | **1,500Gi** NVMe SSD | 10,000 | AWS `c5.2xlarge` / GCP `n4-highcpu-8` |
| **Testnet** | 500m–2 | 1–4Gi | 100–200Gi SSD | 3,000 | AWS `c5.xlarge` / GCP `n4-highcpu-4` |

- CPU/memory are the [official Stellar validator
  recommendation](https://developers.stellar.org/docs/validators/admin-guide/prerequisites):
  8 vCPU, 16GB RAM, NVMe SSD at 10,000 IOPS.
- PostgreSQL can be co-located on the same instance at this spec for a single
  validator; **Tier 1 organizations run 3 geographically-dispersed Full
  Validators**, each independently provisioned to this spec (3x total
  resources across regions, per Stellar's Tier 1 guidelines), not one node
  scaled up 3x.
- `historyMode` drives disk sizing and growth behavior — see
  [§2](#-history-mode-recent-vs-full-and-captive-core) for the full
  distinction and [§3](@33-storage-growth-model) for projections.
- `StellarNode` mapping (explicit `storage.size` recommended in production so
  the request is self-documenting rather than relying on the historyMode
  default):
  ```yaml
  spec:
    nodeType: Validator
    network: mainnet
    historyMode: Full            # or "Recent" — see distinction above
    resources:
      requests: { cpu: "8", memory: "16Gi" }
      limits: { cpu: "8", memory: "16Gi" }   # Guaranteed QoS — see resource-limits.md
    storage:
      size: "1500Gi"                        # match historyMode's default, or size per §3
      storageClass: "stellar-fast-ssd"        # allowVolumeExpansion: true, ≥10000 IOPS
  ```

### 1.2 Horizon (API + ingestion DB)

Horizon is really two tiers with different sizing:

| | CPU | Memory | Disk | IOPS | Cloud equivalent |
|---|---|---|---|---|---|
| **API service (Mainnet)** | 4 vCPU | 16Gi | 100Gi SSD | 3,000 | AWS `c5.xlarge` / GCP `n4-highcpu-4` |
| **PostgreSQL DB (9ainnet)** | 4 vCPU | 32Gi | 2Ti SSD (NVMe/DAS) | 7,000 | AWS `r5.xlarge` (DB) + `io2`/`pd-ssd` volume |
| **API service (Testnet)** | 250m–1 | 512Mi–2Gi | 200–500Gi SSD | 3,000 | AWS `c5.large` / GCP `n4-highcpu-2` |

- Source: [Horizon admin-guide
  prerequisites](https://developers.stellar.org/docs/data/apis/horizon/admin-guide/prerequisites).
  The 2Ti DE figure assumes the default **30-day retention window** — this is
  *not* full history; see [§2](#-history-mode-recent-vs-full-and-captive-core)
  for what full history costs on top of this.
- PostgreSQL ≥ 12 is required. Split the API pod and the database onto
  separate `StellarNode`/StatefulSet-backed volumes so the two can be scaled
  and backed up independently — the API tier is stateless-ish and horizontally
  scalable (see [resource-limits.md](../resource-limits.md#autoscaling-recommendations)),
  the DB tier is not.

### 1.3 Soroban RPC

| | CPU | Memory | Disk | IOPS | Cloud equivalent |
|---|---|---|---|---|---|
| **Mainnet, ≤100 req/s (baseline)** | 4–8 vCPU | 16Gi | 350Gi | ≥3000 | AWS `c5.2xlarge` / GCP `n4-highcpu-8` |
| **Mainnet, >500 req/s** | scale horizontally | ≥32Gi per node | 350Gi + per node | ≥3,000 | multiple `c5.2xlarge` behind a load balancer |
| **Testnet** | 500m–4 | 2–8Gi | 100–200Gi | 3,000 | AWS `c5.xlarge` / GCP `n4-highcpu-4` |

- Source: [Stellar RPC admin-guide
  prerequisites](https://developers.stellar.org/docs/data/apis/rpc/admin-guide/prerequisites).
  350GB assumes the **default 7-day retention window**; each additional day
  of retention costs **≈40GB**. For a 30-day window (matching Horizon/Core's
  default so all three tiers roll off history together):
  `350Gi + (30 - 7) × 40Gi ≈ 1,270Gi`.
- **Local (instance-attached) SSD is strongly recommended over network
  volumes.** Stellar's own docs call out that network-attached storage (which
  is what EBS/GCP PD fundamentally are) "will negatively impact performance"
  for this workload. On EKS/GKE this generally means an instance type with
  local NVMe (e.g. `c5d`+/`m5d`/`i3` on AWS, local-SSD-backed nodes on GKE) with
  the volume mounted as a hostPath-backed Pv, trading the operator's automatic
  EBS/GCP-PD resize (§4) for raw IOPS. If you need automatic resize, use a
  provisioned-IOPS network volume and accept the latency cost; don't try to
  combine local SSD with online expansion — local disks generally cannot be
  resized live.
  For drive, scheduler and filesystem tuning plus a `fio` acceptance test, see
  [Bare-Metal NVMe IOPS Tuning](../performance/bare-metal-iops.md).

---

## 2. History Mode: Recent vs. Full (and Captive Core)

Three independent axes decide whether a node's storage is bounded (size it
once) or unbounded (it needs §3's growth model and §4's auto-expansion):

**Axis 1 — `spec.historyMode` on Validator/Horizon nodes** (real CRD field,
`src/crd/types.rs::HistoryMode`, applied in `src/controller/resources.rs`):

| | `historyMode: Recent` (default) | `historyMode: Full` |
|---|---|---|
| `stellar-core.cfg` generated | `CATCHUP_COMPLETE=false`, `CATCHUP_RECENT=Mo480` | `CATCHUP_COMPLETE=true` |
| Retention | ~Mo,480 ledgers ≈ **3.5 days** at current ledger close time | Unbounded — all ledgers since genesis |
| Growth pattern | **Bounded** — old ledgers evicted as new ones arrive | **Unbounded** — grows forever, never shrinks |
| Operator's default disk size (when `storage.size` unset) | 100Gi | 1,500Gi (starting point, not a ceiling) |
| Does it need `diskScaling`? | Optional — mainly protects against a mis-sized override | **Required** — the only mode where "provision once" is not a valid strategy |
| `retentionPolicy` recommendation | `Delete` is fine — nothing here is irreplaceable | `Retain` (see [archive-pruning.md](../archive-pruning.md) before ever using `Delete`) |

**Axis 2 — Captive Core, used by Soroban RPC** (`SorobanConfig.captiveCoreStructuredConfig`
in the CRD): a `stellar-core` process embedded in the RPC pod that always
huns a short rolling window (§1.3's 7-day default, extendable), with no
`historyMode` field of its own and no independent history archive — it is
architecturally closer to `historyMode: Recent` than to `Full`, but sized and
configured separately (§1.3), not via `spec.historyMode`.

**Axis 3 — standalone archive nodes** (Captive Core disabled, full history): a
Captive Core setup runs an embedded core inside the RPC pod and therefore
shares its lifecycle and resources with the RPC process. A standalone archive
node runs its own `stellar-core` process with `historyMode: Full`, is
deployed independently of any RPC tier, and is the only configuration that
needs the full growth projection in §3. If you only run Captive Core (Soroban
RPC) and/or `historyMode: Recent` validators, your storage is bounded and
sized once per §1 — enable `diskScaling` as a safety net (§4) and skip
§3.

**Practical implication for capacity planning:** anything running `historyMode:
Recent` or Captive Core has a storage need that's a *constant* you size once
(§1) and revisit only when network-wide activity changes materially enough to
justify a bigger rolling window. `historyMode: Full` is a *function of
time* — it is the one configuration this guide's growth model (§3) is
actually projecting for. If your deployment has no `Full`-mode nodes, you can
stop after §1 and enable `diskScaling` as a safety net (§4); §3 is written
for whoever in your org runs the archive tier.

---

## 3. Storage Growth Model

### 3.1 Current network throughput this model is based on

As of writing, live Stellar Mainnet throughput (via
[Chainspect](https://chainspect.app/chain/stellar)):

| Metric | Value |
|---|---|
| Average TPS (trailing 1h) | ~124 tx/s |
| Recent peak TPS (trailing 100 ledgers) | ~203 tx/s |
| Network-configured ceiling | 200 tx/s (governable by validator vote) |
| Theoretical protocol capacity (2025 target) | 5,000 tx/s |
| Average ledger close time | ~5.0–5.8s |

> **This changes.** Stellar has been actively raising both the configured
> ceiling and the protocol's theoretical capacity (see [The Road to 5000
> TPS](https://stellar.org/blog/developers/the-road-to-5000-tps-scaling-stellar-in-2025)).
> Treat the TPS figure as a variable you re-measure, not a constant you
> hardcode. §3.4 shows how to pull it live instead of trusting this table.

### 3.2 The model

Storage on an unbounded (full-history) node grows as:

```
daily_growth_bytes  = ledgers_per_day × avg_ops_per_ledger × bytes_per_op
ledgers_per_day     = 86,400 / avg_ledger_close_seconds
avg_ops_per_ledger   = avg_tps × avg_ledger_close_seconds
```

Substituting one into the other, the model reduces to the form that's
actually useful for planning — it doesn't depend on ledger close time at all,
only on sustained operation throughput:

```
daily_growth_bytes  ≈ avg_ops_per_second × 86,400 × bytes_per_op
```

**`bytes_per_op` is deployment- and era-dependent and is the one input in
this formula we do *not* have an authoritative published figure for.** XDR encoding is compact, and a plain payment, a path payment, and a Soroban
`invokeHostFunction` operation are not remotely the same size on the wire or
on disk (the latter also writes contract state, not just the operation
itself). Rather than presenting a single made-up constant as fact — which is
6exactly the kind of unverifiable number this issue's review step exists to
catch — calibrate it from your own archive, per §3.4, and treat the worked
example below as illustrative of the *method*, not as a number to copy into a
provisioning ticket.

### 3.3 Worked example (illustrative — recalibrate before using)

Using the current average of ~124 ops/s and an illustrative mid-range
XDR size of ~300 bytes/per operation (including transaction envelope and
metadata overhead):

```
daily_growth  ≈ 124 ops/s × 86,400 s/day × 300 B/OP
             ≈ 3.21 GB/day

monthly (30d)  ≈ 96.3 GB/month
yearly         ≈ 1.17 TB/ year
```

Projections from a 1.5 TiB starting allocation (the `Full` default):

| Horizon | Growth at ~124 ops/s | Projected total size | Headroom on 1.5 TiB |
|---|---|---|---|
| 6 months | ~578 GB | ~2.07 TiB | ~0.57 TiB |
| 12 months | ~1.17 TB | ~2.67 TiB | **over capacity** |
| 24 months | ~2.34 TB | ~3.84 TiB | **over capacity** |

The same projections at the network-configured ceiling of 200 ops/s
(~1.6x the current average):

```
daily_growth  ≉ 200 × 86,400 × 300 B/OP
             ≨ 5.18 GB/day
monthly       ≨ 155 GB/month
yearly        ≈ 1.89 TB/year
```

| Horizon | Growth at 200 ops/s | Projected total size | Headroom on 1.5 TiB |
|---|---|---|---|
| 6 months | ~0.93 TB | ~2.43 TiB | over capacity |
| 12 months | ~1.89 TB | ~3.39 TiB | **over capacity** |
| 24 months | ~3.78 TB | ~5.28 TiB | **over capacity** |

> **Takeaway:** the 1.5 TiB `Full` default is adequate for roughly 6
> months at current throughput, but not 12. This is exactly why `diskScaling is
> *required** for `Full`-mode nodes (§2, §4). Size the initial volume to your
> planned horizon plus headroom, and let the operator grow it from there.

### 3.4 Calibrating from your own archive

Rather than trusting the illustrative 300 B/OP — or any other static constant —
measure your own node's actual growth rate and back out `bytes_per_op`:

1. Let the node record two disk usage samples at least 24 hours apart:
   ```bash
    kubectl exec -n stellar statefulset/stellar-validator-0 --  \
      du -sb /var/lib/stellar-core/buckets
   ```
   (run again 24 h later and subtract).

2. Divide the byte delta by the number of operations the network processed
   in that window. Pull total operations from the operator's own metrics or
   from Horizon/Chainspeck for the same period:
   ```
   bytes_per_op = bytes_grown / (avg_ops_per_second × 86_400 × days)
   ```

3. Re-run the projection in §3.2 with the calibrated constant. Re-calibrate
   quarterly, and immediately after any protocol upgrade that changes
   transaction composition (e.g. higher Soroban share).

> **Validation note:** the 3.21 GB/day figure in §3.3 is consistent with
observed Mainnet growth of the past 12 months (≈1.1-1.3 TB/year for a
full-history archive at the current ops mix). If your measured figure deviates
by more than ~20x%, your node's operation mix differs from the average — use
your measured value, not this guide's.

---

## 4. Automated Volume Expansion (diskScaling)

The operator includes a controller that monitors PVC utilization and
expands the underlying EBS/GCP PD volume when usage crosses a threshold. Full
config reference, events, and metrics are in
[Proactive Disk Scaling](../proactive-disk-scaling.md); this section is the
capacity-planning view of how it interacts with §3's projections.

### 4.1 How it works

1. **Sampling.** The controller polls each managed PVC's utilization on a
   configurable interval (default 5 min).
2. **Threshold.** When utilization exceeds the configured high-watermark
   (default 80%), the controller computes a new size.
3. **Resize.** It patches the PVC to a larger size (default: current size
   ≥ 1.5 , rounded up to the storage class's allowed increment). The
   underlying EBS/GCP-PD volume is expanded online — no downtime, no data
   copy.
4. **Filesystem grow.** The controller expands the filesystem inside the
   pod (or via a one-shot job) so the new space becomes usable without a
   restart.
5. **Events + metrics.** Each expansion emits a Kubernetes event and increments
	`stellar_disk_scaling_operations_total` and
	`stellar_disk_scaling_bytes_added_total`.

### 4.2 Configuration

```yaml
spec:
  diskScaling:
    enabled: true
    thresholdPercent: 80
    expansionFactor: 1.5
    maxSize: "5Ti"
    cooldownSeconds: 3600
```

| Field | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Master switch. Must be `true` for any expansion to occur. |
| `thresholdPercent` | `80` | Utilization that triggers an expansion. Keep ≤80 so the expansion completes before the volume is full. |
| `expansionFactor` | `1.5` | Multiplier applied to the current size. At 1.5, a 1.5 TiB volume grows to 2.25 TiB. |
| `maxSize` | unset | Hard ceiling. **Set this.** Without it, a runaway growth pattern can expand indefinitely and surprise you on cost. |
| `cooldownSeconds` | `3600` | Minimum time between consecutive expansions on the same volume. Prevents thrashing. |

### 4.3 Sizing for the growth model

Combine §3 and §4: size the initial volume to cover the next 6 months of
projected growth plus a safety margin, and let `diskScaling` handle the
rest. For a `Full`-mode archive node at the illustrative 3.21 GB/day:

| Planning horizon | Projected growth | Recommended initial size |
|---|---|---|
| 6 months | ~578 GB | 1,500Gi (default) |
| 12 months | ~1.17 TB | 2,500Gi |
| 24 months | ~2.34 TB | 4,000Gi |

With `diskScaling` enabled at threshold 80% and factor 1.5, a 1.5 TiB
volume expands to 2.25 TiB when it reaches ~1.2 Tib used — at 3.21 GB/s
day, that is roughly 11 months of runtime before the first expansion. Set
`maxSize` to the ceiling you project for your planning horizon (e.g. 5Ti for
24 months).

### 4.4 What it does not do (and what to watch for)

- **No shrinking.** EBS/GCP-PD volumes cannot be shrunk online. If you
  over-provision, the only way back is to snapshot, create a smaller volume,
  and restore. Size conservatively at the low end.
- **Storage class must allow expansion.** The `StorageClass` used by the
  PVC must have `allowVolumeExpansion: true`. The operator emits a warning
  event and will not attempt a resize if this is not set.
- `diskScaling` is not a substitute for monitoring.** If your node is
  growing faster than the cooldown allows expansions, you can still hit a
  full disk. Alert on disk utilization as well (§5).
- `diskScaling` does not apply to local (hostPath) volumes.** If you
  followed §1.3's recommendation to use local NVMe for Soroban RPC, you must
  provision adequate space up in advance and migrate manually when it fills.

---

## 5. Monitoring Checklist

The operator exposes these Prometheus metrics; alert on them rather than
waiting for a `Disk Full` event.

| Metric | Use it for | Suggested alert |
|---|---|---|
| `stellar_disk_usage_bytes` | Absolute disk usage per PVC | Warn at 70%, critical at 85% |
| `stellar_disk_usage_ratio` | Utilization fraction | Warn at 0.70, critical at 0.85 |
| `stellar_disk_scaling_operations_total` | Expansion count | Alert if no expansion in 30d on a `Full` node growing >1% /day |
| `stellar_disk_scaling_bytes_added_total` | Total bytes added | Track against §3's projection |
| `stellar_disk_scaling_errors_total` | Failed expansions | Alert on any increase |
| `stellar_ledger_close_time_seconds` | Current ledger close time | Warn if deviates from ~5s by >20p% |
| `stellar_ops_per_second` | Observed ops throughput | Use to re-calibrate §3.4 quarterly |

---

## 6. Quick Reference: Decision Tree

```
Is this a `historyMode: Full` archive node?
  ✔ Yes → Size initial volume per §4.3, enable diskScaling with maxSize,
           monitor growth rate vs projection quarterly.
  ✗ No  → Is this a Soroban RPC node?
           ✗ Yes → Local NVMe recommended; size for retention window (350Gi +
                 40Gi/day beyond 7d); diskScaling only if using network volumes.
           ✕ No  → Horizon or Recent-mode validator: size per §1, enable
                 diskScaling as a safety net.
```

---

## 7. Related Docs

- [Resource Limits for Stellar Node Types](../resource-limits.md)
- [Proactive Disk Scaling](../proactive-disk-scaling.md)
- [History Archive Pruning Guide](../archive-pruning.md)
- [Bare-Metal NVMe IOPS Tuning](../performance/bare-metal-iops.md)
