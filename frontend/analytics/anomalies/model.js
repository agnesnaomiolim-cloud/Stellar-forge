export const MIN_BASELINE_SAMPLES = 8;
export const MAX_CHART_POINTS = 240;

// Welford's algorithm avoids cancellation when measurements have large offsets.
export function statistics(points) {
  let count = 0;
  let mean = 0;
  let m2 = 0;
  for (const { value } of points) {
    count += 1;
    const delta = value - mean;
    mean += delta / count;
    m2 += delta * (value - mean);
  }
  return { count, mean: count ? mean : null, deviation: count > 1 ? Math.sqrt(Math.max(0, m2 / (count - 1))) : 0 };
}

function validPoint(point) {
  return Number.isFinite(point?.timestamp) && Number.isFinite(point?.value);
}

// Buckets retain extrema, including narrow spikes, while bounding SVG work.
function chartPoints(points) {
  if (points.length <= MAX_CHART_POINTS) return points;
  const output = [points[0]];
  // Reserve the endpoints so a recovered/latest sample cannot disappear when
  // it is not an extremum in its bucket.
  const size = Math.ceil(points.length / Math.floor((MAX_CHART_POINTS - 2) / 2));
  for (let start = 0; start < points.length; start += size) {
    let min = points[start];
    let max = min;
    for (let i = start; i < Math.min(start + size, points.length); i += 1) {
      if (points[i].value < min.value) min = points[i];
      if (points[i].value > max.value) max = points[i];
    }
    for (const point of min.timestamp <= max.timestamp ? [min, max] : [max, min]) {
      if (output.at(-1) !== point) output.push(point);
    }
  }
  if (output.at(-1) !== points.at(-1)) output.push(points.at(-1));
  return output;
}

export function analyzeSeries(series, { end, duration, baselineOffset, sigma = 3 }) {
  if (![end, duration, baselineOffset, sigma].every(Number.isFinite)
    || duration <= 0 || baselineOffset < duration || sigma <= 0) {
    throw new Error('Choose finite, non-overlapping comparison windows and a positive sigma.');
  }
  const start = end - duration;
  const baselineEnd = end - baselineOffset;
  const baselineStart = baselineEnd - duration;
  const baseline = [];
  const current = [];
  let discarded = 0;
  // Windows are (start, end], so adjacent ranges never share samples.
  for (const point of series.points ?? []) {
    if (!validPoint(point)) { discarded += 1; continue; }
    if (point.timestamp > start && point.timestamp <= end) current.push(point);
    else if (point.timestamp > baselineStart && point.timestamp <= baselineEnd) baseline.push(point);
  }
  current.sort((a, b) => a.timestamp - b.timestamp);
  baseline.sort((a, b) => a.timestamp - b.timestamp);
  const historical = statistics(baseline);
  const comparison = statistics(current);
  const ready = historical.count >= MIN_BASELINE_SAMPLES;
  const lower = ready ? historical.mean - sigma * historical.deviation : null;
  const upper = ready ? historical.mean + sigma * historical.deviation : null;
  // Tolerance avoids false positives caused by floating point rounding for flat baselines.
  const tolerance = ready ? Math.max(1, Math.abs(historical.mean)) * Number.EPSILON * 16 : 0;
  const breaches = ready ? current.filter(({ value }) => value < lower - tolerance || value > upper + tolerance) : [];
  const latest = current.at(-1)?.value ?? null;
  return {
    id: series.id, label: series.label, unit: series.unit ?? '', historical, comparison,
    lower, upper, latest, breachCount: breaches.length, discarded,
    status: !ready || !current.length ? 'insufficient' : breaches.length ? 'anomaly' : 'normal',
    latestAnomalous: ready && latest !== null && (latest < lower - tolerance || latest > upper + tolerance),
    delta: ready && comparison.count ? comparison.mean - historical.mean : null,
    current: chartPoints(current), baseline: chartPoints(baseline),
    // Retain at most one marker per chart bucket; breach count uses every sample.
    markers: chartPoints(breaches),
    windows: { start, end, baselineStart, baselineEnd },
  };
}

export function analyzeAll(series, options) {
  return series.map((metric) => analyzeSeries(metric, options));
}
