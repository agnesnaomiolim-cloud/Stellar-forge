import { METRICS } from './fixtures.js';

export async function loadSeries({ end, duration, baselineOffset, signal, fetcher = fetch }) {
  // Four bounded cluster aggregates, each with no more than ~1500 samples.
  const start = end - baselineOffset - duration;
  const step = Math.max(15, Math.ceil((end - start) / 1500));
  return Promise.all(METRICS.map(async (metric) => {
    const params = new URLSearchParams({ query: metric.query, start: String(start), end: String(end), step: String(step) });
    const response = await fetcher(`/api/prometheus/api/v1/query_range?${params}`, { signal });
    if (!response.ok) throw new Error(`${metric.label}: HTTP ${response.status}`);
    const payload = await response.json();
    if (payload.status !== 'success' || payload.data?.resultType !== 'matrix') {
      throw new Error(payload.error || `${metric.label}: expected Prometheus range data`);
    }
    const result = payload.data.result;
    if (!Array.isArray(result) || result.length > 1) throw new Error(`${metric.label}: expected one aggregated series`);
    const values = result.length ? result[0]?.values : [];
    if (!Array.isArray(values)) throw new Error(`${metric.label}: expected metric samples`);
    const points = values.map((sample) => {
      // Null and empty strings coerce to zero, which would hide missing data or
      // create a false throughput dip. Reject malformed samples explicitly.
      if (!Array.isArray(sample) || sample.length !== 2
        || typeof sample[0] !== 'number' || !Number.isFinite(sample[0])
        || typeof sample[1] !== 'string' || sample[1].trim() === '') {
        throw new Error(`${metric.label}: malformed metric sample`);
      }
      return { timestamp: sample[0], value: Number(sample[1]) };
    });
    return { ...metric, points };
  }));
}
