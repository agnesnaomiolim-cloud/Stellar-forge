import {
  startTransition,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react';
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

import {
  METRIC_LABELS,
  compareWindowToBaseline,
  createWorkbenchState,
  generateWorkbenchFixture,
  ingestSnapshot,
  seriesChartData,
} from '../../../anomalies/anomalyModel.js';

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/** Polling interval for workbench simulation (ms) */
const POLL_INTERVAL_MS = 3_000;

const METRIC_UNITS = {
  tps: ' tx/s',
  ledgerCloseMs: ' ms',
  p99LatencyMs: ' ms',
  cpuRatio: '',
  memoryMb: ' MB',
  syncLagLedgers: '',
  peerCount: '',
  wasmExecUs: ' µs',
};

const STATUS_COLORS = {
  normal: '#39d98a',
  high: '#f05d5e',
  low: '#f5b942',
  unknown: '#8ea2b2',
};

const STATUS_LABELS = {
  normal: 'Normal',
  high: '⚠ High',
  low: '⚠ Low',
  unknown: '—',
};

// ─────────────────────────────────────────────────────────────────────────────
// State management via useReducer to avoid excessive re-renders
// ─────────────────────────────────────────────────────────────────────────────

function workbenchReducer(state, action) {
  switch (action.type) {
    case 'INGEST_SNAPSHOT':
      return ingestSnapshot(state, action.snapshot);
    case 'SET_SIGMA':
      return { ...state, sigmaThreshold: action.sigmaThreshold };
    default:
      return state;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

function formatValue(value, unit = '') {
  if (value == null) return '—';
  const abs = Math.abs(value);
  if (abs >= 1000) return `${(value / 1000).toFixed(1)}k${unit}`;
  if (abs >= 1) return `${value.toFixed(2)}${unit}`;
  return `${value.toFixed(4)}${unit}`;
}

function formatSigma(sigma) {
  const abs = Math.abs(sigma);
  const sign = sigma >= 0 ? '+' : '−';
  return `${sign}${abs.toFixed(1)}σ`;
}

function formatTimestamp(ts) {
  if (ts == null) return '—';
  return new Date(ts).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function formatPct(value) {
  if (value == null) return '—';
  const sign = value >= 0 ? '+' : '';
  return `${sign}${value.toFixed(1)}%`;
}

// ─────────────────────────────────────────────────────────────────────────────
// Mini AnomalyCard (self-contained to avoid cross-package import issues)
// ─────────────────────────────────────────────────────────────────────────────

function AnomalyCardSparkline({ chartData, statusColor, height = 60 }) {
  // Simple inline sparkline without the full component to avoid tsx import
  return (
    <div style={{ height }} aria-hidden="true">
      <ResponsiveContainer width="100%" height={height}>
        <LineChart data={chartData} margin={{ top: 2, right: 2, left: 0, bottom: 2 }}>
          <XAxis dataKey="timestamp" hide />
          <YAxis hide />
          {chartData.length > 0 && chartData[0]?.upperBound != null && (
            <ReferenceLine
              y={chartData[chartData.length - 1]?.upperBound}
              stroke="#f05d5e"
              strokeDasharray="4 3"
              strokeWidth={1}
            />
          )}
          {chartData.length > 0 && chartData[0]?.lowerBound != null && (
            <ReferenceLine
              y={chartData[chartData.length - 1]?.lowerBound}
              stroke="#f5b942"
              strokeDasharray="4 3"
              strokeWidth={1}
            />
          )}
          {chartData.length > 0 && chartData[0]?.mean != null && (
            <ReferenceLine
              y={chartData[chartData.length - 1]?.mean}
              stroke="#445566"
              strokeDasharray="2 4"
              strokeWidth={1}
            />
          )}
          <Line
            type="monotone"
            dataKey="value"
            stroke={statusColor}
            strokeWidth={2}
            dot={false}
            isAnimationActive={false}
            connectNulls
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

function MetricCard({ metricKey, seriesState, onClick, selected }) {
  const unit = METRIC_UNITS[metricKey] ?? '';
  const { latestStatus, latestSigma, latestValue, baseline, updatedAt, label } = seriesState;
  const isAnomalous = latestStatus === 'high' || latestStatus === 'low';
  const statusColor = STATUS_COLORS[latestStatus];
  const chartData = useMemo(() => seriesChartData(seriesState).slice(-30), [seriesState]);

  const cardClass = [
    'wb-card',
    isAnomalous ? 'wb-card--alert' : '',
    selected ? 'wb-card--selected' : '',
    latestStatus === 'unknown' ? 'wb-card--unknown' : '',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <article
      className={cardClass}
      onClick={onClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onClick();
        }
      }}
      aria-label={`${label} metric card, status: ${STATUS_LABELS[latestStatus]}`}
      aria-pressed={selected}
      aria-live={isAnomalous ? 'assertive' : 'off'}
    >
      <div className="wb-card__header">
        <span className="wb-card__metric-name">{label}</span>
        <span
          className={`wb-card__badge wb-card__badge--${latestStatus}`}
          style={{ color: statusColor }}
          role="status"
        >
          {STATUS_LABELS[latestStatus]}
        </span>
      </div>

      <div className="wb-card__value-row">
        <span
          className="wb-card__value"
          style={{ color: statusColor }}
          aria-label={`Current value: ${formatValue(latestValue, unit)}`}
        >
          {formatValue(latestValue, unit)}
        </span>
        {latestStatus !== 'unknown' && (
          <span
            className={`wb-card__sigma ${latestSigma >= 0 ? 'wb-card__sigma--pos' : 'wb-card__sigma--neg'}`}
            aria-label={`${formatSigma(latestSigma)} standard deviations from baseline`}
          >
            {formatSigma(latestSigma)}
          </span>
        )}
      </div>

      <AnomalyCardSparkline
        chartData={chartData}
        statusColor={statusColor}
        height={60}
      />

      <div className="wb-card__footer">
        {baseline ? (
          <>
            <span>μ {formatValue(baseline.mean, unit)}</span>
            <span>σ {formatValue(baseline.stdDev, unit)}</span>
          </>
        ) : (
          <span className="wb-card__pending">Collecting baseline…</span>
        )}
        {updatedAt && (
          <span className="wb-card__updated">{formatTimestamp(updatedAt)}</span>
        )}
      </div>
    </article>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Detail panel — full time-range comparison view for the selected metric
// ─────────────────────────────────────────────────────────────────────────────

function DetailPanel({ metricKey, seriesState }) {
  const unit = METRIC_UNITS[metricKey] ?? '';
  const { label, baseline } = seriesState;
  const chartData = useMemo(() => seriesChartData(seriesState), [seriesState]);

  const comparison = useMemo(
    () => compareWindowToBaseline(seriesState.samples, seriesState.baseline?.sigmaThreshold ?? 2.5),
    [seriesState],
  );

  function tooltipFormatter(value) {
    if (value == null) return ['—', ''];
    return [`${Number(value).toFixed(3)}${unit}`, ''];
  }

  return (
    <section className="wb-detail" aria-label={`Detail view for ${label}`}>
      <div className="wb-detail__header">
        <h3 className="wb-detail__title">{label}</h3>
        <div className="wb-detail__meta">
          {baseline && (
            <>
              <span>Baseline μ: {formatValue(baseline.mean, unit)}</span>
              <span>σ: {formatValue(baseline.stdDev, unit)}</span>
              <span>n: {baseline.count}</span>
            </>
          )}
        </div>
      </div>

      {/* Time-range comparison summary */}
      <div className="wb-detail__comparison" role="region" aria-label="Time-range comparison">
        <ComparisonTile
          label="Baseline mean"
          value={formatValue(comparison.baselineMean, unit)}
        />
        <ComparisonTile
          label="Recent mean"
          value={formatValue(comparison.recentMean, unit)}
          highlight={comparison.anomalyRate > 0}
        />
        <ComparisonTile
          label="Mean delta"
          value={formatPct(comparison.meanDeltaPct)}
          highlight={Math.abs(comparison.meanDeltaPct ?? 0) > 10}
        />
        <ComparisonTile
          label="Anomaly rate"
          value={`${(comparison.anomalyRate * 100).toFixed(0)}%`}
          highlight={comparison.anomalyRate > 0}
          alertLevel={comparison.anomalyRate > 0.5 ? 'high' : comparison.anomalyRate > 0 ? 'low' : 'normal'}
        />
        <ComparisonTile
          label="Anomalous samples"
          value={`${comparison.anomalyCount} / ${Math.min(seriesState.samples.length, 20)}`}
          highlight={comparison.anomalyCount > 0}
        />
      </div>

      {/* Full history chart */}
      <div className="wb-detail__chart">
        <ResponsiveContainer width="100%" height={260}>
          <LineChart data={chartData} margin={{ top: 8, right: 16, left: 8, bottom: 8 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="#1e2c3a" />
            <XAxis
              dataKey="timestamp"
              tickFormatter={formatTimestamp}
              stroke="#445566"
              minTickGap={40}
            />
            <YAxis stroke="#445566" width={56} />
            <Tooltip
              formatter={tooltipFormatter}
              labelFormatter={(v) => formatTimestamp(v)}
            />
            <Legend />

            {/* Threshold bands */}
            {baseline && (
              <ReferenceLine
                y={baseline.upperBound}
                stroke="#f05d5e"
                strokeDasharray="5 3"
                label={{
                  value: `+${baseline.sigmaThreshold}σ`,
                  position: 'insideTopRight',
                  fill: '#f05d5e',
                  fontSize: 10,
                }}
              />
            )}
            {baseline && (
              <ReferenceLine
                y={baseline.lowerBound}
                stroke="#f5b942"
                strokeDasharray="5 3"
                label={{
                  value: `-${baseline.sigmaThreshold}σ`,
                  position: 'insideBottomRight',
                  fill: '#f5b942',
                  fontSize: 10,
                }}
              />
            )}
            {baseline && (
              <ReferenceLine
                y={baseline.mean}
                stroke="#8ea2b2"
                strokeDasharray="2 5"
                label={{ value: 'μ', position: 'insideTopLeft', fill: '#8ea2b2', fontSize: 10 }}
              />
            )}

            {/* Anomalous points highlighted red */}
            <Line
              type="monotone"
              dataKey="value"
              name={label}
              stroke="#39d98a"
              strokeWidth={2}
              dot={(props) => {
                const { cx, cy, payload } = props;
                if (payload.status === 'normal' || payload.status === 'unknown') return null;
                return (
                  <circle
                    key={`dot-${props.index}`}
                    cx={cx}
                    cy={cy}
                    r={4}
                    fill={payload.status === 'high' ? '#f05d5e' : '#f5b942'}
                    stroke="#0a1520"
                    strokeWidth={1}
                  />
                );
              }}
              isAnimationActive={false}
              connectNulls
            />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </section>
  );
}

function ComparisonTile({ label, value, highlight = false, alertLevel = 'normal' }) {
  const cls = [
    'wb-cmp-tile',
    highlight ? `wb-cmp-tile--${alertLevel}` : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <div className={cls}>
      <span className="wb-cmp-tile__label">{label}</span>
      <strong className="wb-cmp-tile__value">{value}</strong>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// AnomalyWorkbench — main dashboard page
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Anomaly Detection Workbench Dashboard.
 *
 * Displays a grid of metric cards with automated σ-threshold overlays.
 * Metrics breaching statistical bounds are highlighted automatically.
 * A detail panel shows the full time-range comparison for the selected metric.
 *
 * Data is seeded from `generateWorkbenchFixture()` on mount and then
 * simulated live by ingesting new snapshots at POLL_INTERVAL_MS.
 *
 * Issue #127 closes this feature.
 */
export default function AnomalyWorkbench() {
  const metricKeys = useMemo(() => Object.keys(METRIC_LABELS), []);

  const [workbench, dispatch] = useReducer(
    workbenchReducer,
    null,
    () => {
      const state = createWorkbenchState(metricKeys);
      const fixture = generateWorkbenchFixture(60);
      return fixture.reduce(
        (acc, snapshot) => ingestSnapshot(acc, snapshot),
        state,
      );
    },
  );

  const [selectedMetric, setSelectedMetric] = useState(metricKeys[0]);
  const [sigmaThreshold, setSigmaThreshold] = useState(2.5);
  const [filter, setFilter] = useState('all'); // 'all' | 'anomalous' | 'normal'
  const [isPaused, setIsPaused] = useState(false);
  const snapshotIndexRef = useRef(0);

  // Live simulation: ingest rolling synthetic snapshots
  useEffect(() => {
    if (isPaused) return;

    const ticker = setInterval(() => {
      const now = Date.now();
      // Simulate a live metric snapshot with slight random noise
      const live = {};
      const baselines = workbench.series;
      for (const key of metricKeys) {
        const s = baselines[key];
        const baseVal = s.baseline?.mean ?? s.latestValue ?? 100;
        const noise = (Math.random() - 0.5) * (s.baseline?.stdDev ?? baseVal * 0.1) * 2;
        // Occasionally inject a spike (1% per metric per tick)
        const spike = Math.random() < 0.01 ? baseVal * (2 + Math.random()) : 0;
        live[key] = Math.max(0, baseVal + noise + spike);
      }
      const snapshot = { timestamp: now, ...live };

      startTransition(() => {
        dispatch({ type: 'INGEST_SNAPSHOT', snapshot });
      });
    }, POLL_INTERVAL_MS);

    return () => clearInterval(ticker);
    // workbench dependency intentionally omitted — we just need baselines for noise calc
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isPaused, metricKeys]);

  // Update sigma threshold in workbench state
  const handleSigmaChange = useCallback(
    (e) => {
      const val = parseFloat(e.target.value);
      if (Number.isFinite(val) && val > 0) {
        setSigmaThreshold(val);
        dispatch({ type: 'SET_SIGMA', sigmaThreshold: val });
      }
    },
    [],
  );

  // Summary counts
  const { anomalousCount, totalCount } = useMemo(() => {
    const series = Object.values(workbench.series);
    const total = series.length;
    const anomalous = series.filter(
      (s) => s.latestStatus === 'high' || s.latestStatus === 'low',
    ).length;
    return { anomalousCount: anomalous, totalCount: total };
  }, [workbench.series]);

  const visibleMetrics = useMemo(() => {
    return metricKeys.filter((key) => {
      const s = workbench.series[key];
      if (!s) return false;
      if (filter === 'anomalous') return s.latestStatus === 'high' || s.latestStatus === 'low';
      if (filter === 'normal') return s.latestStatus === 'normal';
      return true;
    });
  }, [metricKeys, workbench.series, filter]);

  const selectedSeries = workbench.series[selectedMetric];

  return (
    <main className="workbench" aria-label="Anomaly Detection Workbench">
      {/* ── Header ────────────────────────────────────────────────────── */}
      <header className="workbench__header">
        <div className="workbench__title-group">
          <span className="workbench__eyebrow">STELLAR-K8S ANALYTICS</span>
          <h1 className="workbench__title">Anomaly Detection Workbench</h1>
          <p className="workbench__subtitle">
            Statistical baseline comparison across {totalCount} cluster metrics.
            Welford σ-threshold auto-detection.
          </p>
        </div>

        <div
          className="workbench__summary-badge"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          {anomalousCount === 0 ? (
            <span className="badge badge--green">All metrics normal</span>
          ) : (
            <span className="badge badge--red" role="alert">
              {anomalousCount} metric{anomalousCount !== 1 ? 's' : ''} anomalous
            </span>
          )}
        </div>
      </header>

      {/* ── Controls ──────────────────────────────────────────────────── */}
      <div className="workbench__controls" role="toolbar" aria-label="Workbench controls">
        <label className="wb-control" htmlFor="sigma-range">
          σ threshold:
          <input
            id="sigma-range"
            type="range"
            min="1.0"
            max="4.0"
            step="0.1"
            value={sigmaThreshold}
            onChange={handleSigmaChange}
            aria-valuemin={1}
            aria-valuemax={4}
            aria-valuenow={sigmaThreshold}
            aria-valuetext={`${sigmaThreshold.toFixed(1)} standard deviations`}
          />
          <span className="wb-control__value">{sigmaThreshold.toFixed(1)}σ</span>
        </label>

        <div className="wb-filter-group" role="group" aria-label="Filter metrics">
          {(['all', 'anomalous', 'normal'] ).map((f) => (
            <button
              key={f}
              className={`wb-filter-btn${filter === f ? ' wb-filter-btn--active' : ''}`}
              onClick={() => setFilter(f)}
              aria-pressed={filter === f}
            >
              {f.charAt(0).toUpperCase() + f.slice(1)}
            </button>
          ))}
        </div>

        <button
          className={`wb-pause-btn${isPaused ? ' wb-pause-btn--paused' : ''}`}
          onClick={() => setIsPaused((p) => !p)}
          aria-pressed={isPaused}
          aria-label={isPaused ? 'Resume live data ingestion' : 'Pause live data ingestion'}
        >
          {isPaused ? '▶ Resume' : '⏸ Pause'}
        </button>
      </div>

      {/* ── Metric grid ───────────────────────────────────────────────── */}
      <section
        className="workbench__grid"
        aria-label="Metric card grid"
        aria-live="polite"
        aria-relevant="additions updates"
      >
        {visibleMetrics.length === 0 && (
          <p className="workbench__empty">No metrics match the current filter.</p>
        )}
        {visibleMetrics.map((key) => (
          <MetricCard
            key={key}
            metricKey={key}
            seriesState={workbench.series[key]}
            selected={selectedMetric === key}
            onClick={() => setSelectedMetric(key)}
          />
        ))}
      </section>

      {/* ── Detail panel ──────────────────────────────────────────────── */}
      {selectedSeries && (
        <DetailPanel metricKey={selectedMetric} seriesState={selectedSeries} />
      )}

      {/* ── CSS (injected inline so the dashboard is self-contained) ──── */}
      <style>{STYLES}</style>
    </main>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Styles
// ─────────────────────────────────────────────────────────────────────────────

const STYLES = `
.workbench {
  font-family: 'Inter', system-ui, sans-serif;
  background: #0a1520;
  color: #c8d8e8;
  min-height: 100vh;
  padding: 1.5rem 2rem 4rem;
  box-sizing: border-box;
}

.workbench__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.workbench__eyebrow {
  font-size: 0.68rem;
  font-weight: 600;
  letter-spacing: 0.12em;
  color: #5aa9ff;
  text-transform: uppercase;
  display: block;
  margin-bottom: 0.25rem;
}

.workbench__title {
  font-size: 1.6rem;
  font-weight: 700;
  color: #e8f4ff;
  margin: 0 0 0.25rem;
}

.workbench__subtitle {
  font-size: 0.8rem;
  color: #6b8299;
  margin: 0;
}

.workbench__summary-badge {
  margin-top: 0.5rem;
}

.badge {
  display: inline-block;
  padding: 0.25rem 0.75rem;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 600;
}

.badge--green { background: #0d3a24; color: #39d98a; border: 1px solid #1a6b3e; }
.badge--red   { background: #3a0d0d; color: #f05d5e; border: 1px solid #7a1c1c; }
.badge--blue  { background: #0d2a3a; color: #5aa9ff; border: 1px solid #1a4a6b; }

/* Controls */
.workbench__controls {
  display: flex;
  align-items: center;
  gap: 1rem;
  flex-wrap: wrap;
  margin-bottom: 1.5rem;
  padding: 0.75rem 1rem;
  background: #0e1d2c;
  border-radius: 8px;
  border: 1px solid #1a2d3e;
}

.wb-control {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.8rem;
  color: #8ea2b2;
}
.wb-control input[type=range] { width: 100px; accent-color: #5aa9ff; }
.wb-control__value { color: #e8f4ff; font-weight: 600; min-width: 2.5rem; }

.wb-filter-group { display: flex; gap: 0.25rem; }
.wb-filter-btn {
  padding: 0.3rem 0.65rem;
  border: 1px solid #1a2d3e;
  background: transparent;
  color: #8ea2b2;
  border-radius: 5px;
  font-size: 0.75rem;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.wb-filter-btn:hover { background: #1a2d3e; color: #c8d8e8; }
.wb-filter-btn--active { background: #1a3d5e; color: #5aa9ff; border-color: #5aa9ff; }

.wb-pause-btn {
  padding: 0.3rem 0.75rem;
  border: 1px solid #1a2d3e;
  background: transparent;
  color: #8ea2b2;
  border-radius: 5px;
  font-size: 0.75rem;
  cursor: pointer;
}
.wb-pause-btn--paused { color: #f5b942; border-color: #7a5a0d; }

/* Metric grid */
.workbench__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 1rem;
  margin-bottom: 2rem;
}

.workbench__empty {
  color: #445566;
  grid-column: 1 / -1;
  text-align: center;
  padding: 2rem;
}

/* Metric card */
.wb-card {
  background: #0e1d2c;
  border: 1px solid #1a2d3e;
  border-radius: 10px;
  padding: 0.9rem 1rem 0.75rem;
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s;
}
.wb-card:hover { border-color: #2a4d6e; box-shadow: 0 2px 12px #00000040; }
.wb-card:focus-visible { outline: 2px solid #5aa9ff; }
.wb-card--selected { border-color: #5aa9ff; box-shadow: 0 0 0 2px #5aa9ff33; }
.wb-card--alert { border-color: #7a1c1c; background: #120e0e; }
.wb-card--alert:hover { border-color: #f05d5e; }
.wb-card--unknown { opacity: 0.7; }

.wb-card__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.4rem;
}
.wb-card__metric-name {
  font-size: 0.72rem;
  color: #6b8299;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 60%;
}
.wb-card__badge {
  font-size: 0.65rem;
  font-weight: 600;
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  background: #0a1520;
}
.wb-card__badge--high { background: #3a0d0d; }
.wb-card__badge--low  { background: #3a2a0d; }
.wb-card__badge--normal { color: #39d98a !important; }

.wb-card__value-row {
  display: flex;
  align-items: baseline;
  gap: 0.4rem;
  margin-bottom: 0.5rem;
}
.wb-card__value { font-size: 1.25rem; font-weight: 700; }
.wb-card__sigma {
  font-size: 0.7rem;
  font-weight: 600;
  opacity: 0.8;
}
.wb-card__sigma--pos { color: #f05d5e; }
.wb-card__sigma--neg { color: #f5b942; }

.wb-card__footer {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem 0.75rem;
  font-size: 0.65rem;
  color: #445566;
  margin-top: 0.4rem;
}
.wb-card__updated { margin-left: auto; }
.wb-card__pending { color: #445566; font-style: italic; }

/* Detail panel */
.wb-detail {
  background: #0e1d2c;
  border: 1px solid #1a2d3e;
  border-radius: 10px;
  padding: 1.25rem 1.5rem 1.5rem;
}
.wb-detail__header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem 1rem;
  margin-bottom: 1rem;
}
.wb-detail__title {
  font-size: 1rem;
  font-weight: 700;
  color: #e8f4ff;
  margin: 0;
}
.wb-detail__meta {
  display: flex;
  gap: 1rem;
  font-size: 0.72rem;
  color: #6b8299;
}

/* Comparison tiles */
.wb-detail__comparison {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
  margin-bottom: 1.25rem;
}
.wb-cmp-tile {
  flex: 1 1 100px;
  background: #0a1520;
  border: 1px solid #1a2d3e;
  border-radius: 7px;
  padding: 0.5rem 0.75rem;
}
.wb-cmp-tile--high  { border-color: #7a1c1c; background: #130d0d; }
.wb-cmp-tile--low   { border-color: #7a5a0d; background: #13100d; }
.wb-cmp-tile--normal { border-color: #0d3a24; }
.wb-cmp-tile__label { display: block; font-size: 0.65rem; color: #445566; margin-bottom: 0.2rem; }
.wb-cmp-tile__value { font-size: 0.95rem; font-weight: 700; color: #e8f4ff; }

.wb-detail__chart { margin-top: 0.5rem; }
`;
