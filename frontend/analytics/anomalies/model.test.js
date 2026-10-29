import test from 'node:test';
import assert from 'node:assert/strict';
import { analyzeAll, analyzeSeries, MAX_CHART_POINTS, statistics } from './model.js';
import { syntheticSeries } from './fixtures.js';
import { loadSeries } from './prometheus.js';

const options = { end: 100, duration: 10, baselineOffset: 10, sigma: 3 };
const series = (baseline, current) => ({ id: 'test', label: 'Test', points: [
  ...baseline.map((value, index) => ({ timestamp: 81 + index, value })),
  ...current.map((value, index) => ({ timestamp: 91 + index, value })),
] });

test('sample standard deviation is numerically stable for large measurements', () => {
  const stats = statistics([1e12 + 1, 1e12 + 2, 1e12 + 3].map((value) => ({ value })));
  assert.equal(stats.mean, 1e12 + 2);
  assert.equal(stats.deviation, 1);
});

test('detects positive and negative spikes without including current values in baseline', () => {
  const result = analyzeSeries(series(Array(10).fill(10), [10, 100, -100, 10]), options);
  assert.equal(result.historical.mean, 10);
  assert.equal(result.breachCount, 2);
  assert.equal(result.status, 'anomaly');
  assert.equal(result.latestAnomalous, false);
});

test('constant baseline is normal until a real deviation occurs', () => {
  const result = analyzeSeries(series(Array(10).fill(0), [0, 0]), options);
  assert.equal(result.status, 'normal');
  assert.equal(result.delta, 0);
  assert.equal(result.lower, 0);
});

test('threshold edges are inclusive and sigma adjusts classification', () => {
  const input = series([9, 11, 9, 11, 9, 11, 9, 11], []);
  const deviation = statistics(input.points).deviation;
  input.points.push({ timestamp: 100, value: 10 + 3 * deviation });
  assert.equal(analyzeSeries(input, options).status, 'normal');
  assert.equal(analyzeSeries(input, { ...options, sigma: 2 }).status, 'anomaly');
});

test('missing and nonfinite data are insufficient, never falsely healthy', () => {
  const input = series([10, 10], [12]);
  input.points.push({ timestamp: 100, value: NaN }, { timestamp: Infinity, value: 10 });
  const result = analyzeSeries(input, options);
  assert.equal(result.status, 'insufficient');
  assert.equal(result.discarded, 2);
  assert.equal(result.lower, null);
  assert.equal(analyzeSeries(series(Array(10).fill(10), []), options).status, 'insufficient');
});

test('adjacent windows share no points and future samples are excluded', () => {
  const input = series(Array(10).fill(10), [20]);
  input.points.push({ timestamp: 101, value: 1000 }, { timestamp: 80, value: 1000 });
  const result = analyzeSeries(input, options);
  assert.equal(result.historical.count, 10);
  assert.equal(result.comparison.count, 1);
  assert.equal(result.latest, 20);
});

test('synthetic spikes and dips are flagged for every supported comparison range', () => {
  const end = 200000;
  for (const duration of [900, 3600, 21600]) {
    for (const baselineOffset of [duration, 86400]) {
      const settings = { end, duration, baselineOffset, sigma: 3 };
      const results = analyzeAll(syntheticSeries(end), settings);
      assert.equal(results[0].status, 'anomaly');
      assert.equal(results[1].status, 'normal');
      assert.equal(results[2].status, 'normal');
      // Throughput dip lies 25 minutes ago, outside the 15-minute window.
      assert.equal(results[3].status, duration === 900 ? 'normal' : 'anomaly');
      assert.ok(analyzeAll(syntheticSeries(end, false), settings).every((result) => result.status === 'normal'));
    }
  }
});

test('chart aggregation bounds render work while preserving a single-sample spike', () => {
  const points = Array.from({ length: 20000 }, (_, index) => ({ timestamp: index, value: index === 19990 ? 100 : 10 }));
  const result = analyzeSeries({ id: 'dense', points }, { end: 19999, duration: 10000, baselineOffset: 10000, sigma: 3 });
  assert.ok(result.current.length <= MAX_CHART_POINTS);
  assert.equal(result.breachCount, 1);
  assert.ok(result.current.some((point) => point.value === 100));
  assert.equal(result.current[0].timestamp, 10000);
  assert.equal(result.current.at(-1).timestamp, 19999);
  assert.equal(result.current.at(-1).value, result.latest);
});

test('invalid windows cannot contaminate historical baselines', () => {
  assert.throws(() => analyzeSeries({ points: [] }, { ...options, baselineOffset: 5 }), /non-overlapping/);
  assert.throws(() => analyzeSeries({ points: [] }, { ...options, sigma: 0 }), /positive sigma/);
});

test('Prometheus adapter uses bounded range requests and preserves missing data', async () => {
  const urls = [];
  const result = await loadSeries({ ...options, fetcher: async (url) => {
    urls.push(new URL(url, 'http://localhost'));
    return { ok: true, json: async () => ({ status: 'success', data: { resultType: 'matrix', result: [] } }) };
  } });
  assert.equal(result.length, 4);
  assert.equal(result[0].points.length, 0);
  assert.equal(urls.length, 8);
  assert.ok(urls.every((url) => url.pathname === '/api/prometheus/api/v1/query_range' && Number(url.searchParams.get('step')) >= 15));
});

test('yesterday comparison queries equal-resolution windows without fetching the intervening day', async () => {
  const urls = [];
  const settings = { end: 200000, duration: 900, baselineOffset: 86400 };
  await loadSeries({ ...settings, fetcher: async (url) => {
    urls.push(new URL(url, 'http://localhost'));
    return { ok: true, json: async () => ({ status: 'success', data: { resultType: 'matrix', result: [] } }) };
  } });
  assert.equal(urls.length, 8);
  for (const url of urls) {
    const start = Number(url.searchParams.get('start'));
    const end = Number(url.searchParams.get('end'));
    assert.equal(end - start, 900);
    assert.ok(end === settings.end || end === settings.end - 86400);
    assert.equal(Number(url.searchParams.get('step')), 15);
  }
});

test('adjacent Prometheus windows do not duplicate their shared boundary sample', async () => {
  const results = await loadSeries({ ...options, fetcher: async (url) => {
    const params = new URL(url, 'http://localhost').searchParams;
    const start = Number(params.get('start'));
    const end = Number(params.get('end'));
    return { ok: true, json: async () => ({ status: 'success', data: { resultType: 'matrix', result: [
      { values: [[start, '10'], [end, '10']] },
    ] } }) };
  } });
  assert.deepEqual(results[0].points.map(({ timestamp }) => timestamp), [90, 100]);
  assert.equal(analyzeSeries(results[0], options).historical.count, 1);
  await assert.rejects(loadSeries({ ...options, baselineOffset: 5 }), /non-overlapping/);
});

test('Prometheus errors are surfaced instead of silently replacing data with synthetic values', async () => {
  await assert.rejects(loadSeries({ ...options, fetcher: async () => ({ ok: false, status: 503 }) }), /503/);
  await assert.rejects(loadSeries({ ...options, fetcher: async () => ({ ok: true, json: async () => ({ status: 'error', error: 'query unavailable' }) }) }), /query unavailable/);
});

test('malformed Prometheus samples are rejected instead of coerced to zero', async () => {
  for (const result of [
    [{}], [{ values: null }], [{ values: [[100, null]] }],
    [{ values: [[100, '']] }], [{ values: [[null, '10']] }],
    [{ values: [[100]] }], [{ values: [null] }],
  ]) {
    await assert.rejects(loadSeries({ ...options, fetcher: async () => ({
      ok: true,
      json: async () => ({ status: 'success', data: { resultType: 'matrix', result } }),
    }) }), /expected metric samples|malformed metric sample/);
  }
});

test('Prometheus nonfinite values remain missing and numeric samples retain their timestamps', async () => {
  const results = await loadSeries({ ...options, fetcher: async () => ({
    ok: true,
    json: async () => ({ status: 'success', data: { resultType: 'matrix', result: [
      { values: [[99, '12.5'], [100, 'NaN']] },
    ] } }),
  }) });
  assert.deepEqual(results[0].points[0], { timestamp: 99, value: 12.5 });
  assert.ok(Number.isNaN(results[0].points[1].value));
  const analyzed = analyzeSeries(results[0], options);
  assert.equal(analyzed.discarded, 1);
  assert.equal(analyzed.latest, 12.5);
});
