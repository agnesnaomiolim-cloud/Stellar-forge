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
    const points = (result[0]?.values ?? []).map(([timestamp, value]) => ({ timestamp: Number(timestamp), value: Number(value) }));
    return { ...metric, points };
  }));
}
