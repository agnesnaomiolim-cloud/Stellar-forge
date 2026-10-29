import React, { useEffect, useRef, useState } from 'react';
import AnomalyCard from '../../components/anomaly_card.tsx';
import { syntheticSeries } from './fixtures.js';
import { loadSeries } from './prometheus.js';
import './styles.css';

const date = (seconds) => new Date(seconds * 1000).toLocaleString();

export default function Workbench() {
  const [source, setSource] = useState('synthetic');
  const [duration, setDuration] = useState(3600);
  const [baseline, setBaseline] = useState('previous');
  const [sigma, setSigma] = useState(3);
  const [spikes, setSpikes] = useState(true);
  const [refresh, setRefresh] = useState(0);
  const [state, setState] = useState({ loading: true, results: [], error: null });
  const generation = useRef(0);

  useEffect(() => {
    const id = ++generation.current;
    const controller = new AbortController();
    const end = Math.floor(Date.now() / 1000);
    const options = { end, duration, baselineOffset: baseline === 'previous' ? duration : 86400, sigma };
    setState({ loading: true, results: [], error: null });
    let disposed = false;
    let settled = false;
    let activeWorker;
    let timeout;
    const fail = (error) => {
      if (disposed || settled) return;
      settled = true;
      clearTimeout(timeout);
      controller.abort();
      activeWorker?.terminate();
      setState({ loading: false, results: [], error: error.message || 'Analysis worker failed' });
    };
    // Bound the entire request, including worker startup and analysis.
    timeout = setTimeout(() => fail(new Error('Metric analysis timed out. Refresh to retry.')), 15000);
    (async () => {
      try {
        activeWorker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
        activeWorker.onmessage = ({ data }) => {
          if (disposed || settled || data.id !== generation.current) return;
          if (data.error) fail(new Error(data.error));
          else {
            settled = true;
            clearTimeout(timeout);
            activeWorker.terminate();
            setState({ loading: false, results: data.results, error: null });
          }
        };
        activeWorker.onerror = () => fail(new Error('Anomaly worker unavailable. Check browser worker support and retry.'));
        activeWorker.onmessageerror = () => fail(new Error('Could not read anomaly analysis results. Refresh to retry.'));
        const series = source === 'synthetic' ? syntheticSeries(end, spikes) : await loadSeries({ ...options, signal: controller.signal });
        if (!disposed && !settled) activeWorker.postMessage({ id, series, options });
      } catch (error) { fail(error); }
    })();
    return () => {
      disposed = true;
      controller.abort();
      clearTimeout(timeout);
      activeWorker?.terminate();
    };
  }, [source, duration, baseline, sigma, spikes, refresh]);

  const windows = state.results[0]?.windows;
  return <main className="anomaly-workbench">
    <a href="/">← Analytics home</a>
    <header><p className="eyebrow">STELLAR / OBSERVABILITY</p><h1>Cluster anomaly workbench</h1>
      <p>Compare current metrics with an independent historical baseline.</p></header>
    <section className="anomaly-controls" aria-label="Comparison settings">
      <label>Data source<select value={source} onChange={(event) => setSource(event.target.value)}><option value="synthetic">Synthetic demonstration</option><option value="prometheus">Prometheus</option></select></label>
      <label>Current range<select value={duration} onChange={(event) => setDuration(Number(event.target.value))}><option value={900}>15 minutes</option><option value={3600}>1 hour</option><option value={21600}>6 hours</option></select></label>
      <label>Historical baseline<select value={baseline} onChange={(event) => setBaseline(event.target.value)}><option value="previous">Previous equal range</option><option value="yesterday">Same range yesterday</option></select></label>
      <label>Threshold<select value={sigma} onChange={(event) => setSigma(Number(event.target.value))}>{[2, 3, 4].map((value) => <option key={value} value={value}>±{value} standard deviations</option>)}</select></label>
      {source === 'synthetic' && <label><input type="checkbox" checked={spikes} onChange={(event) => setSpikes(event.target.checked)} /> Inject latency spikes and throughput dip</label>}
      <button onClick={() => setRefresh((value) => value + 1)}>Refresh snapshot</button>
    </section>
    <p className="anomaly-notice">{source === 'synthetic' ? 'Synthetic data — demonstration only.' : 'Prometheus cluster aggregates. Missing exporters produce insufficient data; request failures are shown below.'} At least 8 historical samples required. Both high and low deviations count.</p>
    {windows && <p>Current: {date(windows.start)} – {date(windows.end)}<br />Historical: {date(windows.baselineStart)} – {date(windows.baselineEnd)} (aligned by elapsed time)</p>}
    <p>Blue: current · Grey dashed: historical · Green: baseline ±{sigma}σ · Red dots: breaches</p>
    <div role="status" aria-live="polite">{state.loading ? 'Analyzing metrics…' : state.error ? '' : `${state.results.filter((result) => result.status === 'anomaly').length} anomalous metrics out of ${state.results.length}`}</div>
    {state.error && <p role="alert">{state.error}</p>}
    <section className="anomaly-grid" aria-label="Cluster metrics" aria-busy={state.loading}>{state.results.map((result) => <AnomalyCard key={result.id} result={result} />)}</section>
  </main>;
}
