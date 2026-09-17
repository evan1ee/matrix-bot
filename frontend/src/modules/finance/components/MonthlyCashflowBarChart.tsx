import React, { useState } from 'react';
import { MonthlyCashflowBucket } from '../types';
import { formatMinor } from '../index';

interface MonthlyCashflowBarChartProps {
  data: MonthlyCashflowBucket[];
  currency: string;
  exponent: number;
}

export const MonthlyCashflowBarChart: React.FC<MonthlyCashflowBarChartProps> = ({
  data,
  currency,
  exponent,
}) => {
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);

  if (!data || data.length === 0) {
    return (
      <div className="p-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-lg">
        No monthly transaction history available.
      </div>
    );
  }

  // Calculate scales
  const maxVal = Math.max(
    ...data.map((d) => Math.max(d.income_minor, d.expense_minor)),
    100
  );

  const width = 640;
  const height = 220;
  const paddingLeft = 55;
  const paddingRight = 20;
  const paddingTop = 25;
  const paddingBottom = 35;

  const chartWidth = width - paddingLeft - paddingRight;
  const chartHeight = height - paddingTop - paddingBottom;

  const numBuckets = data.length;
  const slotWidth = chartWidth / numBuckets;
  const barGap = 3;
  const barWidth = Math.max(Math.min((slotWidth - 14) / 2, 22), 6);

  // Y-axis grid ticks (4 intervals)
  const yTicks = [0, 0.33, 0.66, 1.0].map((fraction) => ({
    val: Math.round(maxVal * fraction),
    y: paddingTop + chartHeight - fraction * chartHeight,
  }));

  const activeBucket = hoveredIndex !== null ? data[hoveredIndex] : null;

  return (
    <div className="flex flex-col space-y-2.5">
      {/* Header & Inspection readout */}
      <div className="flex items-center justify-between min-h-[32px] px-1">
        <div>
          <h4 className="text-xs font-medium text-neutral-900 dark:text-neutral-100 font-sans">
            Monthly Inflow vs. Outflow
          </h4>
          <p className="text-[11px] text-neutral-500 font-sans">
            Income and expense side-by-side per billing month
          </p>
        </div>

        {activeBucket ? (
          <div className="flex items-center gap-3 text-xs font-mono">
            <span className="text-neutral-400">{activeBucket.month}</span>
            <span className="text-emerald-600 dark:text-emerald-400">
              +{formatMinor(activeBucket.income_minor, exponent)}
            </span>
            <span className="text-rose-600 dark:text-rose-400">
              −{formatMinor(activeBucket.expense_minor, exponent)}
            </span>
            <span
              className={
                activeBucket.net_minor >= 0
                  ? 'text-neutral-900 dark:text-neutral-100 font-medium'
                  : 'text-rose-600 dark:text-rose-400 font-medium'
              }
            >
              Net {activeBucket.net_minor >= 0 ? '+' : ''}
              {formatMinor(activeBucket.net_minor, exponent)} {currency}
            </span>
          </div>
        ) : (
          <div className="flex items-center gap-4 text-xs font-mono text-neutral-500">
            <div className="flex items-center gap-1.5">
              <span className="w-2.5 h-2.5 rounded-xs bg-emerald-500 dark:bg-emerald-400" />
              <span>Income</span>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="w-2.5 h-2.5 rounded-xs bg-rose-500 dark:bg-rose-400" />
              <span>Expense</span>
            </div>
          </div>
        )}
      </div>

      {/* SVG Column Chart */}
      <div className="relative w-full overflow-hidden rounded-lg bg-neutral-50/50 dark:bg-neutral-950/50 border border-neutral-200 dark:border-neutral-800 p-2">
        <svg
          viewBox={`0 0 ${width} ${height}`}
          className="w-full h-auto select-none"
          role="img"
          aria-label="Monthly income vs expense column chart"
        >
          {/* Grid lines & Y tick labels */}
          {yTicks.map((tick, i) => (
            <g key={i}>
              <line
                x1={paddingLeft}
                y1={tick.y}
                x2={width - paddingRight}
                y2={tick.y}
                className="stroke-neutral-200 dark:stroke-neutral-850"
                strokeDasharray="3 3"
                strokeWidth="1"
              />
              <text
                x={paddingLeft - 8}
                y={tick.y + 3}
                textAnchor="end"
                className="text-[9px] font-mono fill-neutral-400 dark:fill-neutral-500"
              >
                {formatMinor(tick.val, exponent)}
              </text>
            </g>
          ))}

          {/* Baseline */}
          <line
            x1={paddingLeft}
            y1={paddingTop + chartHeight}
            x2={width - paddingRight}
            y2={paddingTop + chartHeight}
            className="stroke-neutral-300 dark:stroke-neutral-700"
            strokeWidth="1"
          />

          {/* Data Buckets */}
          {data.map((bucket, idx) => {
            const slotCenterX = paddingLeft + idx * slotWidth + slotWidth / 2;
            const incomeHeight = (bucket.income_minor / maxVal) * chartHeight;
            const expenseHeight = (bucket.expense_minor / maxVal) * chartHeight;

            const incomeX = slotCenterX - barWidth - barGap / 2;
            const expenseX = slotCenterX + barGap / 2;

            const incomeY = paddingTop + chartHeight - incomeHeight;
            const expenseY = paddingTop + chartHeight - expenseHeight;

            const isHovered = hoveredIndex === idx;

            return (
              <g
                key={bucket.month}
                onMouseEnter={() => setHoveredIndex(idx)}
                onMouseLeave={() => setHoveredIndex(null)}
                className="cursor-pointer"
              >
                {/* Hover backdrop track */}
                <rect
                  x={paddingLeft + idx * slotWidth}
                  y={paddingTop}
                  width={slotWidth}
                  height={chartHeight}
                  fill={isHovered ? 'currentColor' : 'transparent'}
                  className="text-neutral-500/5 dark:text-neutral-100/5 transition-colors"
                />

                {/* Income Bar */}
                <rect
                  x={incomeX}
                  y={incomeY}
                  width={barWidth}
                  height={Math.max(incomeHeight, 1)}
                  rx="2"
                  className={`transition-opacity ${
                    isHovered ? 'opacity-100' : 'opacity-85'
                  } fill-emerald-600 dark:fill-emerald-400`}
                />

                {/* Expense Bar */}
                <rect
                  x={expenseX}
                  y={expenseY}
                  width={barWidth}
                  height={Math.max(expenseHeight, 1)}
                  rx="2"
                  className={`transition-opacity ${
                    isHovered ? 'opacity-100' : 'opacity-85'
                  } fill-rose-500 dark:fill-rose-400`}
                />

                {/* X-axis Month Label */}
                <text
                  x={slotCenterX}
                  y={paddingTop + chartHeight + 16}
                  textAnchor="middle"
                  className={`text-[10px] font-mono transition-colors ${
                    isHovered
                      ? 'fill-neutral-900 dark:fill-neutral-100 font-medium'
                      : 'fill-neutral-500 dark:fill-neutral-400'
                  }`}
                >
                  {bucket.month.slice(2)}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
    </div>
  );
};
