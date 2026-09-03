import React, { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { TrendingUp, TrendingDown, Calendar } from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
import { PortfolioHistoryResponse, PortfolioHistoryPoint } from './types';

interface PerformanceChartProps {
  currency: string;
  currencyExponent: number;
}

const TIMEFRAMES = [
  { label: '7D', value: '7d' },
  { label: '1M', value: '1m' },
  { label: '3M', value: '3m' },
  { label: '1Y', value: '1y' },
  { label: 'ALL', value: 'all' },
];

function formatAmount(minor: number, exponent: number): string {
  const full = (minor / 10 ** exponent).toFixed(exponent);
  return Number(full).toLocaleString(undefined, {
    minimumFractionDigits: Math.min(exponent, 2),
    maximumFractionDigits: Math.min(exponent, 2),
  });
}

export const PerformanceChart: React.FC<PerformanceChartProps> = ({
  currency,
  currencyExponent,
}) => {
  const [timeframe, setTimeframe] = useState('1m');
  const [hoveredPoint, setHoveredPoint] = useState<PortfolioHistoryPoint | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ['portfolio-history', timeframe, currency],
    queryFn: () =>
      fetchApi<PortfolioHistoryResponse>(
        `/portfolio/history?timeframe=${timeframe}&currency=${encodeURIComponent(currency)}`
      ),
    staleTime: 60_000,
  });

  const points = data?.points ?? [];
  const isPositive = (data?.period_change_minor ?? 0) >= 0;

  // Chart dimensions
  const width = 650;
  const height = 180;
  const paddingX = 20;
  const paddingTop = 20;
  const paddingBottom = 25;

  const innerWidth = width - paddingX * 2;
  const innerHeight = height - paddingTop - paddingBottom;

  // Compute min/max for scale
  const values = points.map((p: PortfolioHistoryPoint) => p.total_net_worth_minor);
  const minVal = values.length > 0 ? Math.min(...values) : 0;
  const maxVal = values.length > 0 ? Math.max(...values) : 1;
  const range = maxVal - minVal > 0 ? maxVal - minVal : 1;

  // Coordinates mapping
  const coords = points.map((p: PortfolioHistoryPoint, i: number) => {
    const x =
      points.length <= 1
        ? paddingX + innerWidth / 2
        : paddingX + (i / (points.length - 1)) * innerWidth;
    const y = paddingTop + innerHeight - ((p.total_net_worth_minor - minVal) / range) * innerHeight;
    return { x, y, point: p };
  });

  // Construct smooth SVG path using Catmull-Rom or cubic Bezier
  const linePath = coords.reduce(
    (
      acc: string,
      curr: { x: number; y: number; point: PortfolioHistoryPoint },
      i: number,
      arr: { x: number; y: number; point: PortfolioHistoryPoint }[]
    ) => {
      if (i === 0) return `M ${curr.x} ${curr.y}`;
      const prev = arr[i - 1];
      const cp1x = prev.x + (curr.x - prev.x) / 2;
      const cp1y = prev.y;
      const cp2x = prev.x + (curr.x - prev.x) / 2;
      const cp2y = curr.y;
      return `${acc} C ${cp1x} ${cp1y}, ${cp2x} ${cp2y}, ${curr.x} ${curr.y}`;
    },
    ''
  );

  const areaPath =
    coords.length > 0
      ? `${linePath} L ${coords[coords.length - 1].x} ${height - paddingBottom} L ${coords[0].x} ${
          height - paddingBottom
        } Z`
      : '';

  // Active display point (either hovered or the latest point)
  const activePoint = hoveredPoint ?? points[points.length - 1];
  const displayTotal = activePoint ? activePoint.total_net_worth_minor : data?.current_value_minor ?? 0;
  const displayChange = data?.period_change_minor ?? 0;
  const displayPct = data?.period_change_pct ?? 0;

  const strokeColor = isPositive ? '#10b981' : '#f43f5e';
  const gradientStart = isPositive ? 'rgba(16, 185, 129, 0.25)' : 'rgba(244, 63, 94, 0.25)';

  const handleMouseMove = (e: React.MouseEvent<SVGSVGElement>) => {
    if (coords.length === 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const mouseX = ((e.clientX - rect.left) / rect.width) * width;

    // Find nearest point
    let nearest = coords[0];
    let minDist = Math.abs(coords[0].x - mouseX);
    for (let i = 1; i < coords.length; i++) {
      const dist = Math.abs(coords[i].x - mouseX);
      if (dist < minDist) {
        minDist = dist;
        nearest = coords[i];
      }
    }
    setHoveredPoint(nearest.point);
  };

  return (
    <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-xl p-4 sm:p-5 shadow-xs flex flex-col justify-between h-full space-y-3">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 font-mono">
              Net Worth History ({currency})
            </h3>
            {hoveredPoint && (
              <span className="flex items-center gap-1 text-[11px] text-neutral-400 font-mono bg-neutral-100 dark:bg-neutral-800 px-1.5 py-0.5 rounded">
                <Calendar className="w-3 h-3" />
                {hoveredPoint.date}
              </span>
            )}
          </div>
          <div className="flex items-baseline gap-2 mt-1">
            <span className="text-xl sm:text-2xl font-bold text-neutral-900 dark:text-neutral-100 font-sans">
              {formatAmount(displayTotal, currencyExponent)} {currency}
            </span>
            <span
              className={`flex items-center gap-0.5 text-xs font-semibold font-mono ${
                isPositive ? 'text-emerald-600 dark:text-emerald-400' : 'text-rose-600 dark:text-rose-400'
              }`}
            >
              {isPositive ? <TrendingUp className="w-3.5 h-3.5" /> : <TrendingDown className="w-3.5 h-3.5" />}
              {isPositive ? '+' : ''}
              {formatAmount(displayChange, currencyExponent)} ({isPositive ? '+' : ''}
              {displayPct.toFixed(2)}%)
            </span>
          </div>
        </div>

        {/* Timeframe Buttons */}
        <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-800 p-1 rounded-lg self-start sm:self-auto">
          {TIMEFRAMES.map((t) => (
            <button
              key={t.value}
              onClick={() => {
                setTimeframe(t.value);
                setHoveredPoint(null);
              }}
              className={`px-2.5 py-1 rounded text-xs font-mono transition cursor-pointer ${
                timeframe === t.value
                  ? 'bg-white dark:bg-neutral-900 text-neutral-900 dark:text-neutral-100 font-bold shadow-xs'
                  : 'text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200'
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>
      </div>

      {/* SVG Chart */}
      <div className="relative w-full overflow-hidden flex-1 flex items-center justify-center">
        {isLoading && (
          <div className="absolute inset-0 flex items-center justify-center bg-white/60 dark:bg-neutral-900/60 z-10">
            <span className="text-xs text-neutral-400 font-mono">Loading chart…</span>
          </div>
        )}

        <svg
          viewBox={`0 0 ${width} ${height}`}
          className="w-full h-36 sm:h-44 overflow-visible select-none"
          onMouseMove={handleMouseMove}
          onMouseLeave={() => setHoveredPoint(null)}
        >
          <defs>
            <linearGradient id="areaGradient" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={gradientStart} />
              <stop offset="100%" stopColor="transparent" />
            </linearGradient>
          </defs>

          {/* Area Fill */}
          {areaPath && <path d={areaPath} fill="url(#areaGradient)" />}

          {/* Line Path */}
          {linePath && (
            <path
              d={linePath}
              fill="none"
              stroke={strokeColor}
              strokeWidth={2.5}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          )}

          {/* Hover Crosshair & Indicator Point */}
          {hoveredPoint && (() => {
            const match = coords.find((c: { point: PortfolioHistoryPoint }) => c.point.date === hoveredPoint.date);
            if (!match) return null;
            return (
              <g>
                <line
                  x1={match.x}
                  y1={paddingTop}
                  x2={match.x}
                  y2={height - paddingBottom}
                  stroke="currentColor"
                  className="text-neutral-300 dark:text-neutral-700"
                  strokeWidth={1}
                  strokeDasharray="3 3"
                />
                <circle
                  cx={match.x}
                  cy={match.y}
                  r={5}
                  fill={strokeColor}
                  className="stroke-white dark:stroke-neutral-900"
                  strokeWidth={2}
                />
              </g>
            );
          })()}

          {/* Bottom Date Labels */}
          {coords.length > 1 && (
            <g className="text-[10px] font-mono fill-neutral-400">
              <text x={paddingX} y={height - 6} textAnchor="start">
                {coords[0].point.date}
              </text>
              <text x={width - paddingX} y={height - 6} textAnchor="end">
                {coords[coords.length - 1].point.date}
              </text>
            </g>
          )}
        </svg>
      </div>

      {/* Detail Breakdown for Active / Hovered Point */}
      {activePoint && (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800/80 text-xs">
          <div>
            <span className="text-[10px] uppercase font-mono text-neutral-400 block">Invested Holdings</span>
            <span className="font-semibold text-neutral-800 dark:text-neutral-200 font-mono">
              {formatAmount(activePoint.holdings_value_minor, currencyExponent)} {currency}
            </span>
          </div>
          <div>
            <span className="text-[10px] uppercase font-mono text-neutral-400 block">Available Cash</span>
            <span className="font-semibold text-neutral-800 dark:text-neutral-200 font-mono">
              {formatAmount(activePoint.cash_minor, currencyExponent)} {currency}
            </span>
          </div>
          <div className="col-span-2 sm:col-span-1">
            <span className="text-[10px] uppercase font-mono text-neutral-400 block">Total Value</span>
            <span className="font-bold text-neutral-900 dark:text-neutral-100 font-mono">
              {formatAmount(activePoint.total_net_worth_minor, currencyExponent)} {currency}
            </span>
          </div>
        </div>
      )}
    </div>
  );
};
