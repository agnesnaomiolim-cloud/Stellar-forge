export const METRICS = [
  { id: 'latency', label: 'p99 request latency', unit: 'ms', query: 'histogram_quantile(0.99, sum by (le) (rate(http_request_duration_seconds_bucket[5m]))) * 1000', center: 80 },
  { id: 'cpu', label: 'Cluster CPU usage', unit: 'cores', query: 'sum(rate(container_cpu_usage_seconds_total{container!="",container!="POD"}[5m]))', center: 12 },
  { id: 'memory', label: 'Cluster working memory', unit: 'GiB', query: 'sum(container_memory_working_set_bytes{container!="",container!="POD"}) / 1073741824', center: 48 },
  { id: 'throughput', label: 'HTTP request throughput', unit: 'req/s', query: 'sum(rate(http_requests_total[5m]))', center: 450 },
];

// Deterministic 49-hour history; last-hour latency spike and throughput dip.
export function syntheticSeries(end, spikes = true) {
  return METRICS.map((metric, metricIndex) => ({
    ...metric,
    points: Array.from({ length: 2941 }, (_, index) => {
      const timestamp = end - (2940 - index) * 60;
      let value = metric.center * (1 + 0.025 * Math.sin(index * 0.47 + metricIndex));
      if (spikes && index >= 2935 && metricIndex === 0) value *= 3;
      if (spikes && index === 2915 && metricIndex === 3) value *= 0.3;
      return { timestamp, value };
    }),
  }));
}
