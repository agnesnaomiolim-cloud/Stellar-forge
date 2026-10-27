# Cluster metric anomaly workbench

Open `/anomalies.html` in the existing analytics Vite application. The analytics
home page links to this independent entry; production builds include it. Default
data is explicitly labelled synthetic. Switch to Prometheus for real cluster
aggregates through the existing `/api/prometheus` proxy to port 9091. Configure
the four PromQL selectors in `fixtures.js` for your exporters; absent metrics
produce insufficient-data cards. Fetch errors remain visible and never switch
to demonstration data. Refresh captures a new snapshot; the dashboard does not
poll automatically.

Choose a 15-minute, one-hour, or six-hour current window, then compare with the
previous equal window or the same range yesterday. Windows are `(start, end]`
and do not overlap. Historical sample mean and sample standard deviation define
the selected ±2σ/3σ/4σ bounds. At least eight finite historical values and one
current value are required. A card highlights if **any** current sample breaches
the bounds, even when the latest value has recovered. Both spikes and dips are
detected. A flat baseline flags any material change, with floating point tolerance.
These are statistical deviations, not automatic incident or causation judgments.

The plots align historical and current windows by elapsed time. Dashed grey
shows history, blue shows current values, green shows statistical bounds, and
red dots show breaches. Raw values are included in the accessible card summary.
Synthetic mode injects a recent p99 latency spike and an earlier throughput dip;
the dip appears only in ranges longer than 15 minutes. Turn off injection to
compare the normal series.

Statistical evaluation, sorting, breach detection, and chart reduction run in a
dedicated module worker. A changed control terminates prior work and ignores
stale results. Prometheus requests are limited to four aggregate series with
approximately 1,500 samples each. SVG output is bounded to 240 extrema-preserving
points per line. Every sample still participates in calculations and breach counts.

A 15-second deadline covers fetching, worker startup, and analysis. Failures abort
outstanding requests and terminate the worker; Refresh retries the full snapshot.
Malformed Prometheus samples produce a visible error rather than being coerced to
zero. Prometheus `NaN` values remain missing samples during analysis.

## Deferred validation

Per `task.txt`, no dependencies, application code, builds, or tests were executed
during implementation. Source and diffs were reviewed only. No UI screenshot or
runtime performance claim has been fabricated. Before merge, run from
`frontend/analytics` in an authorized validation session:

```sh
node --test anomalies/model.test.js anomalies/component.test.js
npm run build:anomalies
npm run dev
```

Capture real screenshots at `/anomalies.html` with injection on and off, and
with the historical comparison switched to yesterday. Confirm worker requests
are cancelled when changing controls rapidly. Check live data and error states
against a Prometheus instance with matching exporters. The analytics topology
entry has also been repaired to remove its duplicate declaration, restore its
dashboard JSX and imports, and select an actual connected view on startup.
Validate the complete application with `npm run build`; `build:anomalies`
selects only the workbench entry and writes to `dist-anomalies`.

Model tests cover numerical stability, threshold boundaries, flat baselines,
positive/negative spikes, recovered series, empty/nonfinite samples, independent
windows, invalid configurations, all supported synthetic ranges, chart reduction,
and Prometheus failures. Component tests transform and server-render the actual
TSX card using the existing Vite/React dependencies and assert calculated alert,
normal, recovered, missing-data, and chart-marker states. Browser interaction and
visual verification remain explicitly pending.
