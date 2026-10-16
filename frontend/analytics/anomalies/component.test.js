import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';
import { analyzeAll, analyzeSeries } from './model.js';
import { syntheticSeries } from './fixtures.js';

test('real TSX card renders calculated anomalies, bounds, recovery, and missing-data states', async () => {
  // Vite transforms the actual TSX component; no duplicate test-only rendering implementation.
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    server: { middlewareMode: true },
    resolve: { dedupe: ['react', 'react-dom'] },
  });
  try {
    const { default: Card } = await server.ssrLoadModule(fileURLToPath(new URL('../../components/anomaly_card.tsx', import.meta.url)));
    const end = 200000;
    const settings = { end, duration: 3600, baselineOffset: 3600, sigma: 3 };
    const results = analyzeAll(syntheticSeries(end), settings);
    const anomalous = renderToStaticMarkup(React.createElement(Card, { result: results[0] }));
    assert.match(anomalous, /data-status="anomaly"/);
    assert.match(anomalous, /Anomaly detected/);
    assert.match(anomalous, /<circle/);
    assert.match(anomalous, /Statistical bounds/);
    assert.match(anomalous, /Historical mean/);
    const recovered = renderToStaticMarkup(React.createElement(Card, { result: results[3] }));
    assert.match(recovered, /Anomaly detected/);
    assert.match(recovered, /Latest: within bounds/);
    const normal = renderToStaticMarkup(React.createElement(Card, { result: results[1] }));
    assert.match(normal, /data-status="normal"/);
    assert.doesNotMatch(normal, /<circle/);
    const missing = analyzeSeries({ id: 'missing', label: 'Unavailable metric', points: [] }, settings);
    const empty = renderToStaticMarkup(React.createElement(Card, { result: missing }));
    assert.match(empty, /Insufficient data/);
    assert.match(empty, /No data/);
    assert.doesNotMatch(empty, /NaN|Infinity|Within baseline/);
  } finally {
    await server.close();
  }
});
