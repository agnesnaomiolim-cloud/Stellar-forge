import React, { useRef, useEffect, useState } from 'react';
import type { VirtualLogListProps, LogLine, LogPatternRule } from './types';

/** Virtualized list for rendering thousands of log lines without browser lag.
 * Inspired by the EventTable virtualization pattern in the codebase.
 * Uses fixed row height and absolute positioning to only render visible + overscanned rows.
 */
const ROW_HEIGHT = 28; // px — must match CSS .log-row height
const OVERSANN = 6; // number of rows to render above/below viewport

export function VirtualLogList({ 
  logLines, 
  patternRules, 
  onLineClick,
  autoScroll 
}: VirtualLogListProps) {
  const containerRef = useRef<HTMLDivElement>(null!);
  
  // Calculate visible window based on scroll position
  const [win, setWin] = useState<{ start: number; end: number; offsetY: number; totalHeight: number }>({
    start: 0,
    end: Math.min(logLines.length, 30),
    offsetY: 0,
    totalHeight: Math.max(0, logLines.length - 1) * ROW_HEIGHT,
  });

  // Recalculate visible window on mount and when log count changes
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const recalc = () => {
      const scrollTop = el.scrollTop;
      const viewportH = el.clientHeight;
      const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSANN);
      const visibleCount = Math.ceil(viewportH / ROW_HEIGHT) + OVERSANN * 2;
      const end = Math.min(logLines.length, start + visibleCount);

      setWin({
        start,
        end,
        offsetY: start * ROW_HEIGHT,
        totalHeight: Math.max(0, logLines.length - 1) * ROW_HEIGHT,
      });
    };

    recalc();
    el.addEventListener('scroll', recalc, { passive: true });

    const ro = new ResizeObserver(recalc);
    ro.observe(el);
    return () => {
      el.removeEventListener('scroll', recalc);
      ro.disconnect();
    };
  }, [logLines.length]);

  // Recalc whenever logLines count changes
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const scrollTop = el.scrollTop;
    const viewportH = el.clientHeight;
    const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSANN);
    const visibleCount = Math.ceil(viewportH / ROW_HEIGHT) + OVERSANN * 2;
    const end = Math.min(logLines.length, start + visibleCount);

    setWin({
      start,
      end,
      offsetY: start * ROW_HEIGHT,
      totalHeight: Math.max(0, logLines.length - 1) * ROW_HEIGHT,
    });
  }, [logLines.length]);

  const slice = logLines.slice(win.start, win.end);

  return (
    <div className="bg-slate-900 border border-slate-700 rounded-lg overflow-hidden">
      {/* Column headers */}
      <div className="p-2 border-b border-slate-700 flex items-center text-xs text-slate-400">
        <span className="flex-1">Timestamp</span>
        <span className="w-20 text-center">Level</span>
        <span className="flex-1">Message</span>
      </div>

      {/* Scrollable virtual list */}
      <div
        ref={containerRef}
        className="event-scroll log-scroll"
        role="grid"
        aria-rowcount={logLines.length}
        aria-label="Stellar-core logs"
        style={{
          height: '400px',
          overflow: 'auto',
          padding: '4px 0',
        }}
        tabIndex={-1}
      >
        {logLines.length === 0 ? (
          <div className="p-4 text-slate-500 text-center">
            <span className="text-xl">⬢</span>
            <p>Waiting for logs…</p>
            <p className="text-slate-400">Real-time stellar-core logs will appear here.</p>
          </div>
        ) : (
          <div
            style={{
              height: win.totalHeight,
              position: 'relative',
            }}
          >
            {slice.map((line, i) => (
              <LogRow
                key={line.id}
                index={i}
                line={line}
                patternRules={patternRules}
                onClick={() => onLineClick?.(line)}
                startOffset={win.offsetY}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** Single log row with pattern-based highlighting. */
interface LogRowProps {
  line: LogLine;
  patternRules: LogPatternRule[];
  onClick: () => void;
  isSelected: boolean;
  index: number;
  startOffset: number;
}

const LogRow: React.FC<LogRowProps> = ({ line, patternRules, onClick, isSelected, index, startOffset }) => {
  // Determine if this line matches any pattern
  const matchingRule = patternRules.find(rule => rule.regex.test(line.message));
  const hasHighlight = !!matchingRule;
  const highlightClass = matchingRule?.highlightClass ?? 'bg-emerald-500/20';

  const handleClick = () => onClick();

  // Format the display message - truncate if too long
  const displayMessage = line.message.length > 150 
    ? `${line.message.substring(0, 150)}…`
    : line.message;

  return (
    <div
      role="row"
      className={`log-row log-row--${line.level}${
        isSelected ? ' log-row--selected' : ''
      } ${hasHighlight ? ' has-highlight' : ''}`}
      style={{
        height: ROW_HEIGHT,
        cursor: 'pointer',
        position: 'relative',
      }}
      onClick={handleClick}
    >
      {/* Timestamp */}
      <span className="log-cell log-cell--timestamp" style={{ width: '120px' }}>
        {new Date(line.timestamp).toLocaleTimeString()}
      </span>

      {/* Level badge */}
      <span
        className={`log-cell log-cell--level badge badge--${line.level}`}
        style={{ width: '70px' }}
      >
        {line.level.toUpperCase()}
      </span>

      {/* Message */}
      <span className="log-cell log-cell--message" style={{ flex: '1', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {displayMessage}
      </span>
    </div>
  );
};