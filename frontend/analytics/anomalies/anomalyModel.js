// Anomaly Detection model for the Stellar-K8s workbench.
//
// All functions are pure and synchronous — no DOM, no React, no I/O.
// Statistical evaluations run in O(n) per metric series so even hundreds
// of cluster metrics remain well inside a single animation frame.
//
// Strategy: Welford online algorithm for rolling mean/variance, then flag
// any sample that deviates by more than `sigmaThreshold` standard deviations
// from the rolling baseline.

/** Default sigma multiplier. Values outside ±N·σ are anomalous. */
export const DEFAULT_SIGMA_THRESHOLD = 2.5;

/** Minimum sample count required before anomaly detection activates. */
export const MIN_HISTORY_SAMPLES = 8;

/** Maximum series samples retained in memory per metric. */
export const MAX_SERIES_SAMPLES = 2000;

/** How many recent samples define the "current" window for comparison. */
export const RECENT_WINDOW = 20;

/** Built-in Stellar-K8s metric labels for the workbench. */
export const METRIC_LABELS = {
  tps: 'Transactions per second',
  ledgerCloseMs: 'Ledger close time (ms)',
  p99LatencyMs: 'p99 Latency (ms)',
  cpuRatio: 'CPU utilisation',
  memoryMb: 'Memory usage (MB)',
  syncLagLedgers: 'Sync lag (ledgers)',
  peerCount: 'Peer count',
  wasmExecUs: 'Wasm execution (µs)',
};

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

function asNumber(value, fallback = null) {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

/**
 * Welford single-pass online algorithm for mean and variance.
 * Returns { mean, variance, stdDev, count }.
 */
export function welford(values) {
  let count = 0;
  let mean = 0;
  let m2 = 0;

  for (const v of values) {
    const n = asNumber(v, null);
    if (n === null) continue;
    count += 1;
    const delta = n - mean;
    mean += delta / count;
    const delta2 = n - mean;
    m2 += delta * delta2;
  }

  const variance = count > 1 ? m2 / (count - 1) : 0;
  return { mean, variance, stdDev: Math.sqrt(variance), count };
}

/**
 * Compute baseline statistics from a window of historical samples.
 * Returns `null` when fewer than MIN_HISTORY_SAMPLES values are available.
 */
export function computeBaseline(samples, sigmaThreshold = DEFAULT_SIGMA_THRESHOLD) {
  const values = samples.map((s) => asNumber(s?.value, null)).filter((v) => v !== null);
  if (values.length < MIN_HISTORY_SAMPLES) return null;

  const { mean, stdDev, count } = welford(values);
  const upperBound = mean + sigmaThreshold * stdDev;
  const lowerBound = mean - sigmaThreshold * stdDev;

  return {
    mean,
    stdDev,
    count,
    sigmaThreshold,
    upperBound,
    lowerBound,
  };
}

/**
 * Classify a single value against a pre-computed baseline.
 *
 * Returns one of:
 *   'normal'   — within bounds
 *   'high'     — above upper bound
 *   'low'      — below lower bound
 *   'unknown'  — no baseline yet
 */
export function classifyAnomaly(value, baseline) {
  if (baseline === null) return 'unknown';
  const n = asNumber(value, null);
  if (n === null) return 'unknown';
  if (n > baseline.upperBound) return 'high';
  if (n < baseline.lowerBound) return 'low';
  return 'normal';
}

/**
 * How many sigma away is `value` from `baseline.mean`?
 * Returns 0 when stdDev is 0 or no baseline.
 */
export function sigmaDeviation(value, baseline) {
  if (!baseline || baseline.stdDev === 0) return 0;
  const n = asNumber(value, null);
  if (n === null) return 0;
  return (n - baseline.mean) / baseline.stdDev;
}

// ---------------------------------------------------------------------------
// Series management
// ---------------------------------------------------------------------------

/**
 * Create an empty metric series state.
 * @param {string} metricKey - canonical key, e.g. 'tps'
 * @param {string} [label] - human-readable label
 */
export function createSeriesState(metricKey, label) {
  return {
    metricKey,
    label: label ?? METRIC_LABELS[metricKey] ?? metricKey,
    samples: [],
    baseline: null,
    latestStatus: 'unknown',
    latestSigma: 0,
    latestValue: null,
    updatedAt: null,
  };
}

/**
 * Ingest a new data point into a series state, updating the baseline and
 * anomaly status. Returns the next (immutable) state object.
 *
 * @param {object} state      - existing series state from `createSeriesState`
 * @param {object} dataPoint  - { value: number, timestamp: number }
 * @param {number} [sigmaThreshold]
 */
export function ingestDataPoint(
  state,
  dataPoint,
  sigmaThreshold = DEFAULT_SIGMA_THRESHOLD,
) {
  const value = asNumber(dataPoint?.value, null);
  const timestamp = asNumber(dataPoint?.timestamp, Date.now());
  if (value === null) return state;

  const sample = { value, timestamp };

  // Append + cap
  const samples = [...state.samples, sample];
  while (samples.length > MAX_SERIES_SAMPLES) samples.shift();

  const baseline = computeBaseline(samples, sigmaThreshold);
  const latestStatus = classifyAnomaly(value, baseline);
  const latestSigma = sigmaDeviation(value, baseline);

  return {
    ...state,
    samples,
    baseline,
    latestStatus,
    latestSigma,
    latestValue: value,
    updatedAt: timestamp,
  };
}

/**
 * Ingest an array of data points into a series state.
 * Points are expected in ascending timestamp order.
 */
export function ingestBatch(state, dataPoints, sigmaThreshold = DEFAULT_SIGMA_THRESHOLD) {
  let next = state;
  for (const point of dataPoints) {
    next = ingestDataPoint(next, point, sigmaThreshold);
  }
  return next;
}

// ---------------------------------------------------------------------------
// Workbench state (multi-metric)
// ---------------------------------------------------------------------------

/**
 * Create a fresh workbench state for a set of metric keys.
 */
export function createWorkbenchState(metricKeys = Object.keys(METRIC_LABELS)) {
  const series = {};
  for (const key of metricKeys) {
    series[key] = createSeriesState(key);
  }
  return { series, sigmaThreshold: DEFAULT_SIGMA_THRESHOLD };
}

/**
 * Ingest a multi-metric snapshot into the workbench state.
 *
 * `snapshot` shape: { timestamp: number, tps?: number, ledgerCloseMs?: number, ... }
 *
 * Returns next workbench state.
 */
export function ingestSnapshot(workbenchState, snapshot) {
  const { timestamp, ...values } = snapshot;
  const ts = asNumber(timestamp, Date.now());
  const series = { ...workbenchState.series };

  for (const key of Object.keys(values)) {
    const value = asNumber(values[key], null);
    if (value === null) continue;
    if (!series[key]) {
      series[key] = createSeriesState(key);
    }
    series[key] = ingestDataPoint(
      series[key],
      { value, timestamp: ts },
      workbenchState.sigmaThreshold,
    );
  }

  return { ...workbenchState, series };
}

// ---------------------------------------------------------------------------
// Time-range comparison (current window vs historical baseline)
// ---------------------------------------------------------------------------

/**
 * Compare a recent window of samples against the full history baseline.
 *
 * Returns:
 *   {
 *     baselineMean, baselineStdDev,
 *     recentMean, recentStdDev,
 *     meanDelta,         // absolute difference
 *     meanDeltaPct,      // percentage change from baseline
 *     anomalyCount,      // samples in recent window that are anomalous
 *     anomalyRate,       // fraction 0–1
 *   }
 */
export function compareWindowToBaseline(samples, sigmaThreshold = DEFAULT_SIGMA_THRESHOLD) {
  if (samples.length < MIN_HISTORY_SAMPLES) {
    return {
      baselineMean: null,
      baselineStdDev: null,
      recentMean: null,
      recentStdDev: null,
      meanDelta: null,
      meanDeltaPct: null,
      anomalyCount: 0,
      anomalyRate: 0,
    };
  }

  const recent = samples.slice(-RECENT_WINDOW);
  const historical = samples.slice(0, -RECENT_WINDOW);
  if (historical.length < MIN_HISTORY_SAMPLES) {
    // Not enough history; treat all-but-recent as history
    const stats = welford(samples.map((s) => s.value));
    const recentStats = welford(recent.map((s) => s.value));
    const baseline = computeBaseline(samples, sigmaThreshold);
    const anomalyCount = recent.filter(
      (s) => classifyAnomaly(s.value, baseline) !== 'normal',
    ).length;
    return {
      baselineMean: stats.mean,
      baselineStdDev: stats.stdDev,
      recentMean: recentStats.mean,
      recentStdDev: recentStats.stdDev,
      meanDelta: recentStats.mean - stats.mean,
      meanDeltaPct: stats.mean !== 0 ? ((recentStats.mean - stats.mean) / Math.abs(stats.mean)) * 100 : null,
      anomalyCount,
      anomalyRate: anomalyCount / recent.length,
    };
  }

  const baselineStats = welford(historical.map((s) => s.value));
  const recentStats = welford(recent.map((s) => s.value));
  const historicalBaseline = computeBaseline(historical, sigmaThreshold);
  const anomalyCount = recent.filter(
    (s) => classifyAnomaly(s.value, historicalBaseline) !== 'normal',
  ).length;
  const meanDelta = recentStats.mean - baselineStats.mean;
  const meanDeltaPct =
    baselineStats.mean !== 0
      ? (meanDelta / Math.abs(baselineStats.mean)) * 100
      : null;

  return {
    baselineMean: baselineStats.mean,
    baselineStdDev: baselineStats.stdDev,
    recentMean: recentStats.mean,
    recentStdDev: recentStats.stdDev,
    meanDelta,
    meanDeltaPct,
    anomalyCount,
    anomalyRate: anomalyCount / recent.length,
  };
}

// ---------------------------------------------------------------------------
// Chart-ready data for rendering
// ---------------------------------------------------------------------------

/**
 * Returns chart data for a single metric series: an array of objects ready
 * for Recharts with upperBound and lowerBound reference lines baked in.
 *
 * Shape per item:
 *   { timestamp, value, upperBound, lowerBound, status }
 */
export function seriesChartData(seriesState) {
  const { samples, baseline } = seriesState;
  return samples.map((s) => ({
    timestamp: s.timestamp,
    value: s.value,
    upperBound: baseline ? baseline.upperBound : null,
    lowerBound: baseline ? baseline.lowerBound : null,
    mean: baseline ? baseline.mean : null,
    status: classifyAnomaly(s.value, baseline),
  }));
}

// ---------------------------------------------------------------------------
// Mock / fixture generator
// ---------------------------------------------------------------------------

/**
 * Generate a synthetic metric time series with injected artificial spikes.
 *
 * @param {object} opts
 * @param {string}  opts.metric        - metric key
 * @param {number}  [opts.length]      - total number of data points (default 60)
 * @param {number}  [opts.baseMean]    - baseline mean value
 * @param {number}  [opts.baseStdDev] - baseline noise std dev
 * @param {number[]} [opts.spikeAt]   - indices where spikes are injected
 * @param {number}  [opts.spikeMultiplier] - spike magnitude as multiple of baseMean (default 3)
 * @param {number}  [opts.startTimestamp] - epoch ms start
 * @param {number}  [opts.stepMs]      - ms between samples (default 60_000)
 */
export function generateSyntheticSeries({
  metric = 'tps',
  length = 60,
  baseMean = 150,
  baseStdDev = 10,
  spikeAt = [],
  spikeMultiplier = 3,
  startTimestamp = Date.now() - (length - 1) * 60_000,
  stepMs = 60_000,
} = {}) {
  // Deterministic pseudo-random using a simple LCG so tests are repeatable.
  let seed = 1234567;
  function lcg() {
    seed = (seed * 1664525 + 1013904223) & 0xffffffff;
    return (seed >>> 0) / 4294967296;
  }
  // Box-Muller for Gaussian noise
  function gauss(mean, stdDev) {
    const u1 = Math.max(1e-10, lcg());
    const u2 = lcg();
    const z0 = Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
    return mean + z0 * stdDev;
  }

  const spikeSet = new Set(spikeAt);
  return Array.from({ length }, (_, i) => ({
    timestamp: startTimestamp + i * stepMs,
    value: spikeSet.has(i)
      ? baseMean * spikeMultiplier + gauss(0, baseStdDev)
      : Math.max(0, gauss(baseMean, baseStdDev)),
    metric,
    isInjectedSpike: spikeSet.has(i),
  }));
}

/**
 * Generate a full multi-metric snapshot fixture suitable for testing the
 * workbench. Returns an array of snapshot objects (one per timestep).
 */
export function generateWorkbenchFixture(length = 60, spikeConfig = {}) {
  const defaults = {
    tps: { baseMean: 150, baseStdDev: 12, spikeAt: [40, 41], spikeMultiplier: 3 },
    ledgerCloseMs: { baseMean: 180, baseStdDev: 15, spikeAt: [20], spikeMultiplier: 2.5 },
    p99LatencyMs: { baseMean: 45, baseStdDev: 5, spikeAt: [50, 51, 52], spikeMultiplier: 4 },
    cpuRatio: { baseMean: 0.35, baseStdDev: 0.05, spikeAt: [30], spikeMultiplier: 2.2 },
    memoryMb: { baseMean: 512, baseStdDev: 30, spikeAt: [], spikeMultiplier: 2 },
    syncLagLedgers: { baseMean: 2, baseStdDev: 0.5, spikeAt: [10, 11], spikeMultiplier: 8 },
    peerCount: { baseMean: 12, baseStdDev: 1, spikeAt: [], spikeMultiplier: 2 },
    wasmExecUs: { baseMean: 2500, baseStdDev: 200, spikeAt: [45], spikeMultiplier: 5 },
  };
  const config = { ...defaults, ...spikeConfig };
  const startTimestamp = Date.now() - (length - 1) * 60_000;

  // Build per-metric arrays then zip into snapshots
  const perMetric = {};
  for (const [key, opts] of Object.entries(config)) {
    perMetric[key] = generateSyntheticSeries({ metric: key, length, startTimestamp, ...opts });
  }

  return Array.from({ length }, (_, i) => {
    const snapshot = { timestamp: startTimestamp + i * 60_000 };
    for (const [key, series] of Object.entries(perMetric)) {
      snapshot[key] = series[i].value;
    }
    return snapshot;
  });
}
