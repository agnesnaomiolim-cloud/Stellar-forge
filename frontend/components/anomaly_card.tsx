import React, { memo } from 'react';

type Point = { timestamp: number; value: number };
type Summary = { count: number; mean: number | null; deviation: number };
export interface AnomalyResult {
  id: string; label: string; unit: string;
  status: 'insufficient' | 'anomaly' | 'normal';
  latest: number | null; latestAnomalous: boolean; breachCount: number;
  historical: Summary; comparison: Summary; lower: number | null; upper: number | null;
  delta: number | null; current: Point[]; baseline: Point[]; markers: Point[];
  windows: { start: number; end: number; baselineStart: number; baselineEnd: number };
}

const format = (value: number | null) => value === null ? 'No data' : value.toLocaleString(undefined, { maximumFractionDigits: 2 });

export default memo(function AnomalyCard({ result }: { result: AnomalyResult }) {
  const { current, baseline, windows, lower, upper } = result;
  let min = Math.min(lower ?? Infinity, upper ?? Infinity);
  let max = Math.max(lower ?? -Infinity, upper ?? -Infinity);
  for (const point of [...current, ...baseline]) {
    min = Math.min(min, point.value);
    max = Math.max(max, point.value);
  }
  if (!Number.isFinite(min)) { min = 0; max = 1; }
  const padding = Math.max((max - min) * 0.12, Math.abs(max) * 0.01, 0.01);
  min -= padding;
  max += padding;
  const y = (value: number) => 105 - (value - min) / (max - min) * 95;
  const x = (timestamp: number, start: number) => 10 + (timestamp - start) / (windows.end - windows.start) * 300;
  const path = (points: Point[], start: number) => points.map((point) => `${x(point.timestamp, start)},${y(point.value)}`).join(' ');
  const label = result.status === 'insufficient' ? 'Insufficient data' : result.status === 'anomaly' ? 'Anomaly detected' : 'Within baseline';
  return <article className={`anomaly-card ${result.status}`} aria-label={`${result.label}: ${label}`} data-status={result.status}>
    <header><h2>{result.label}</h2><span className="anomaly-badge">{label}</span></header>
    <p className="anomaly-value">{format(result.latest)} <small>{result.unit}</small></p>
    <p>{result.breachCount} out-of-bounds samples · Latest: {result.status === 'insufficient' ? 'not evaluated' : result.latestAnomalous ? 'outside bounds' : 'within bounds'}</p>
    <svg viewBox="0 0 320 120" role="img" aria-label={`${result.label}: current in blue, historical in grey, statistical bounds shaded`}>
      <title>{result.label} time range comparison</title>
      {lower !== null && upper !== null && <g>
        <rect x="10" width="300" y={y(upper)} height={Math.max(1, y(lower) - y(upper))} fill="#265d47" opacity="0.35" />
        {[lower, upper].map((bound, index) => <line key={index} x1="10" x2="310" y1={y(bound)} y2={y(bound)} stroke="#73bb94" strokeDasharray="4 3" />)}
      </g>}
      <polyline points={path(baseline, windows.baselineStart)} fill="none" stroke="#9aa8ba" strokeWidth="1.5" strokeDasharray="4 3" />
      <polyline points={path(current, windows.start)} fill="none" stroke="#67bcff" strokeWidth="2" />
      {result.markers.map((point, index) => <circle key={index} cx={x(point.timestamp, windows.start)} cy={y(point.value)} r="3" fill="#ff8e86" />)}
    </svg>
    <dl>
      <div><dt>Historical mean ± standard deviation</dt><dd>{format(result.historical.mean)} ± {format(result.historical.deviation)} {result.unit}</dd></div>
      <div><dt>Statistical bounds</dt><dd>{format(lower)} – {format(upper)} {result.unit}</dd></div>
      <div><dt>Current mean / change</dt><dd>{format(result.comparison.mean)} / {result.delta === null ? 'No comparison' : `${result.delta >= 0 ? '+' : ''}${format(result.delta)}`} {result.unit}</dd></div>
      <div><dt>Samples (historical / current)</dt><dd>{result.historical.count} / {result.comparison.count}</dd></div>
    </dl>
  </article>;
});
