import {
  Area,
  AreaChart,
  CartesianGrid,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

export type AnomalyStatus = 'normal' | 'high' | 'low' | 'unknown';

export interface AnomalyChartPoint {
  /** Unix epoch milliseconds */
  timestamp: number;
  value: number | null;
  /** Upper sigma bound at this point */
  upperBound: number | null;
  /** Lower sigma bound at this point */
  lowerBound: number | null;
  /** Rolling mean */
  mean: number | null;
  /** 'normal' | 'high' | 'low' | 'unknown' */
  status: AnomalyStatus;
}

export interface AnomalyCardProps {
  /** Metric display name */
  label: string;
  /** Metric key (e.g. 'tps') — used as aria label suffix */
  metricKey: string;
  /** Most recent value */
  latestValue: number | null;
  /** Anomaly classification of the latest value */
  latestStatus: AnomalyStatus;
  /** How many σ the latest value deviates from the baseline mean */
  latestSigma: number;
  /** Chart data for the sparkline (recent window) */
  chartData: AnomalyChartPoint[];
  /** Baseline mean (for tooltip context) */
  baselineMean: number | null;
  /** Baseline σ (for tooltip context) */
  baselineStdDev: number | null;
  /** Optional unit suffix, e.g. "ms", "MB" */
  unit?: string;
  /** ISO-8601 or epoch-ms of last ingest */
  updatedAt?: number | null;
  /** Sparkline height in px (default 80) */
  height?: number;
  /** Called when the card is clicked, e.g. to drill down */
  onClick?: () => void;
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

const STATUS_COLORS: Record<AnomalyStatus, string> = {
  normal: '#39d98a',
  high: '#f05d5e',
  low: '#f5b942',
  unknown: '#8ea2b2',
};

const STATUS_LABELS: Record<AnomalyStatus, string> = {
  normal: 'Normal',
  high: '⚠ High anomaly',
  low: '⚠ Low anomaly',
  unknown: 'Insufficient data',
};

function formatValue(value: number | null, unit = ''): string {
  if (value === null) return '—';
  const abs = Math.abs(value);
  if (abs >= 1000) return `${(value / 1000).toFixed(1)}k${unit}`;
  if (abs >= 1) return `${value.toFixed(2)}${unit}`;
  return `${value.toFixed(4)}${unit}`;
}

function formatTimestamp(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function formatSigma(sigma: number): string {
  const abs = Math.abs(sigma);
  const sign = sigma >= 0 ? '+' : '−';
  return `${sign}${abs.toFixed(1)}σ`;
}

// ─────────────────────────────────────────────────────────────────────────────
// Tooltip
// ─────────────────────────────────────────────────────────────────────────────

interface TooltipPayloadItem {
  dataKey: string;
  value: number | null;
  name: string;
  color: string;
}

interface SparklineTooltipProps {
  active?: boolean;
  payload?: TooltipPayloadItem[];
  label?: number;
  unit?: string;
}

function SparklineTooltip({ active, payload, label, unit = '' }: SparklineTooltipProps) {
  if (!active || !payload?.length) return null;

  const valueEntry = payload.find((p) => p.dataKey === 'value');
  const upperEntry = payload.find((p) => p.dataKey === 'upperBound');
  const lowerEntry = payload.find((p) => p.dataKey === 'lowerBound');

  return (
    <div className="anomaly-card__tooltip" role="tooltip">
      {label != null && (
        <p className="anomaly-card__tooltip-time">{formatTimestamp(label)}</p>
      )}
      {valueEntry?.value != null && (
        <p>
          <span style={{ color: valueEntry.color }}>●</span>{' '}
          Value: <strong>{formatValue(valueEntry.value, unit)}</strong>
        </p>
      )}
      {upperEntry?.value != null && (
        <p className="anomaly-card__tooltip-bound">
          Upper bound: {formatValue(upperEntry.value, unit)}
        </p>
      )}
      {lowerEntry?.value != null && (
        <p className="anomaly-card__tooltip-bound">
          Lower bound: {formatValue(lowerEntry.value, unit)}
        </p>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// AnomalyCard
// ─────────────────────────────────────────────────────────────────────────────

/**
 * AnomalyCard — a compact metric card with a sparkline and statistical
 * threshold overlays. Highlights automatically when the latest value breaches
 * the Welford-computed sigma bounds.
 *
 * Used by the Anomaly Detection Workbench dashboard (issue #127).
 */
export function AnomalyCard({
  label,
  metricKey,
  latestValue,
  latestStatus,
  latestSigma,
  chartData,
  baselineMean,
  baselineStdDev,
  unit = '',
  updatedAt = null,
  height = 80,
  onClick,
}: AnomalyCardProps) {
  const isAnomalous = latestStatus === 'high' || latestStatus === 'low';
  const statusColor = STATUS_COLORS[latestStatus];

  const cardClass = [
    'anomaly-card',
    isAnomalous ? 'anomaly-card--alert' : '',
    latestStatus === 'unknown' ? 'anomaly-card--unknown' : '',
    onClick ? 'anomaly-card--clickable' : '',
  ]
    .filter(Boolean)
    .join(' ');

  // Area fill uses a faint version of the status color
  const areaFill = isAnomalous ? `${statusColor}22` : '#39d98a22';

  return (
    <article
      className={cardClass}
      aria-label={`${label} anomaly card`}
      aria-live={isAnomalous ? 'assertive' : 'polite'}
      aria-atomic="true"
      onClick={onClick}
      role={onClick ? 'button' : 'article'}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onClick();
              }
            }
          : undefined
      }
    >
      {/* Header */}
      <div className="anomaly-card__header">
        <span className="anomaly-card__label" title={metricKey}>
          {label}
        </span>
        <span
          className={`anomaly-card__status anomaly-card__status--${latestStatus}`}
          role="status"
          aria-label={STATUS_LABELS[latestStatus]}
        >
          {STATUS_LABELS[latestStatus]}
        </span>
      </div>

      {/* Value + sigma */}
      <div className="anomaly-card__value-row">
        <span
          className="anomaly-card__value"
          style={{ color: statusColor }}
          aria-label={`Current value: ${formatValue(latestValue, unit)}`}
        >
          {formatValue(latestValue, unit)}
        </span>
        {latestStatus !== 'unknown' && (
          <span
            className={`anomaly-card__sigma anomaly-card__sigma--${latestSigma >= 0 ? 'pos' : 'neg'}`}
            aria-label={`${formatSigma(latestSigma)} standard deviations from baseline`}
          >
            {formatSigma(latestSigma)}
          </span>
        )}
      </div>

      {/* Sparkline with threshold overlays */}
      <div
        className="anomaly-card__chart"
        aria-hidden="true"
        style={{ height }}
      >
        <ResponsiveContainer width="100%" height={height}>
          <AreaChart
            data={chartData}
            margin={{ top: 2, right: 4, left: 0, bottom: 2 }}
          >
            <CartesianGrid strokeDasharray="3 3" stroke="#1e2c3a" />
            <XAxis
              dataKey="timestamp"
              tickFormatter={formatTimestamp}
              stroke="#445566"
              tick={false}
              axisLine={false}
            />
            <YAxis
              stroke="#445566"
              tick={false}
              axisLine={false}
              width={0}
            />
            <Tooltip
              content={<SparklineTooltip unit={unit} />}
              cursor={{ stroke: '#445566', strokeDasharray: '3 3' }}
            />

            {/* Upper bound reference */}
            {chartData.length > 0 && chartData[0].upperBound != null && (
              <ReferenceLine
                y={chartData[chartData.length - 1]?.upperBound ?? undefined}
                stroke="#f05d5e"
                strokeDasharray="4 3"
                strokeWidth={1}
                label={{
                  value: `+${(baselineStdDev != null ? 2.5 : 0).toFixed(1)}σ`,
                  position: 'insideTopRight',
                  fill: '#f05d5e',
                  fontSize: 9,
                }}
              />
            )}

            {/* Lower bound reference */}
            {chartData.length > 0 && chartData[0].lowerBound != null && (
              <ReferenceLine
                y={chartData[chartData.length - 1]?.lowerBound ?? undefined}
                stroke="#f5b942"
                strokeDasharray="4 3"
                strokeWidth={1}
              />
            )}

            {/* Mean reference */}
            {baselineMean != null && (
              <ReferenceLine
                y={baselineMean}
                stroke="#8ea2b2"
                strokeDasharray="2 4"
                strokeWidth={1}
              />
            )}

            {/* Upper + lower bound area lines */}
            <Area
              type="monotone"
              dataKey="upperBound"
              stroke="#f05d5e"
              strokeWidth={1}
              strokeDasharray="4 3"
              fill="none"
              dot={false}
              isAnimationActive={false}
              connectNulls
              legendType="none"
            />
            <Area
              type="monotone"
              dataKey="lowerBound"
              stroke="#f5b942"
              strokeWidth={1}
              strokeDasharray="4 3"
              fill="none"
              dot={false}
              isAnimationActive={false}
              connectNulls
              legendType="none"
            />

            {/* Main metric area */}
            <Area
              type="monotone"
              dataKey="value"
              stroke={statusColor}
              strokeWidth={2}
              fill={areaFill}
              dot={false}
              isAnimationActive={false}
              connectNulls
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>

      {/* Footer: baseline stats */}
      <div className="anomaly-card__footer">
        {baselineMean != null ? (
          <>
            <span>μ {formatValue(baselineMean, unit)}</span>
            {baselineStdDev != null && (
              <span>σ {formatValue(baselineStdDev, unit)}</span>
            )}
          </>
        ) : (
          <span className="anomaly-card__footer-pending">
            Collecting baseline…
          </span>
        )}
        {updatedAt != null && (
          <span className="anomaly-card__updated">
            {formatTimestamp(updatedAt)}
          </span>
        )}
      </div>
    </article>
  );
}

export default AnomalyCard;
