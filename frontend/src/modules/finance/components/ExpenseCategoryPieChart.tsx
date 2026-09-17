import React, { useState } from 'react';
import { ExpenseCategorySlice } from '../types';
import { formatMinor } from '../index';

interface ExpenseCategoryPieChartProps {
  categories: ExpenseCategorySlice[];
  currency: string;
  exponent: number;
  totalExpenseMinor: number;
}

// Restrained harmonic palette adhering to Design.md
const PALETTE = [
  '#09090b', // neutral-950 (or light neutral-100 in dark)
  '#71717a', // zinc-500
  '#f43f5e', // rose-500
  '#f59e0b', // amber-500
  '#3b82f6', // blue-500
  '#8b5cf6', // violet-500
  '#14b8a6', // teal-500
  '#a1a1aa', // zinc-400
];

const DARK_PALETTE = [
  '#f4f4f5', // zinc-100
  '#a1a1aa', // zinc-400
  '#fb7185', // rose-400
  '#fbbf24', // amber-400
  '#60a5fa', // blue-400
  '#a78bfa', // violet-400
  '#2dd4bf', // teal-400
  '#71717a', // zinc-500
];

export const ExpenseCategoryPieChart: React.FC<ExpenseCategoryPieChartProps> = ({
  categories,
  currency,
  exponent,
  totalExpenseMinor,
}) => {
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);

  if (!categories || categories.length === 0 || totalExpenseMinor <= 0) {
    return (
      <div className="p-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-lg">
        No expense category records for this period.
      </div>
    );
  }

  // Geometry for SVG Donut
  const size = 220;
  const cx = size / 2;
  const cy = size / 2;
  const outerR = 92;
  const innerR = 58;

  // Compute angles for each slice
  let currentAngle = -Math.PI / 2; // Start from top 12 o'clock
  const slices = categories.map((cat, index) => {
    const fraction = cat.percentage / 100;
    const sweepAngle = Math.max(fraction * 2 * Math.PI, 0.001);
    const startAngle = currentAngle;
    const endAngle = currentAngle + sweepAngle;
    currentAngle = endAngle;

    // Outer arc endpoints
    const x1 = cx + outerR * Math.cos(startAngle);
    const y1 = cy + outerR * Math.sin(startAngle);
    const x2 = cx + outerR * Math.cos(endAngle);
    const y2 = cy + outerR * Math.sin(endAngle);

    // Inner arc endpoints
    const ix1 = cx + innerR * Math.cos(endAngle);
    const iy1 = cy + innerR * Math.sin(endAngle);
    const ix2 = cx + innerR * Math.cos(startAngle);
    const iy2 = cy + innerR * Math.sin(startAngle);

    const largeArcFlag = sweepAngle > Math.PI ? 1 : 0;

    // SVG path description
    const d = [
      `M ${x1} ${y1}`,
      `A ${outerR} ${outerR} 0 ${largeArcFlag} 1 ${x2} ${y2}`,
      `L ${ix1} ${iy1}`,
      `A ${innerR} ${innerR} 0 ${largeArcFlag} 0 ${ix2} ${iy2}`,
      'Z',
    ].join(' ');

    return {
      cat,
      index,
      d,
      color: PALETTE[index % PALETTE.length],
      darkColor: DARK_PALETTE[index % DARK_PALETTE.length],
    };
  });

  const activeSlice = hoveredIdx !== null ? slices[hoveredIdx] : null;

  return (
    <div className="flex flex-col space-y-3">
      <div>
        <h4 className="text-xs font-medium text-neutral-900 dark:text-neutral-100 font-sans">
          Expense Allocation by Category
        </h4>
        <p className="text-[11px] text-neutral-500 font-sans">
          Proportional breakdown of total expenditure
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-12 gap-5 items-center rounded-lg bg-neutral-50/50 dark:bg-neutral-950/50 border border-neutral-200 dark:border-neutral-800 p-4">
        {/* SVG Donut Chart */}
        <div className="md:col-span-5 flex flex-col items-center justify-center relative">
          <svg
            viewBox={`0 0 ${size} ${size}`}
            className="w-48 h-48 sm:w-52 sm:h-52 select-none"
            role="img"
            aria-label="Expense breakdown donut chart"
          >
            {slices.map((s) => {
              const isHovered = hoveredIdx === s.index;
              return (
                <path
                  key={s.cat.category_name}
                  d={s.d}
                  fill={s.color}
                  stroke="var(--color-canvas, #ffffff)"
                  strokeWidth="2"
                  className={`transition-all duration-150 cursor-pointer ${
                    isHovered ? 'opacity-100 scale-[1.02] origin-center' : 'opacity-85 hover:opacity-100'
                  }`}
                  style={{
                    fill: 'var(--slice-color)',
                  }}
                  data-category={s.cat.category_name}
                  onMouseEnter={() => setHoveredIdx(s.index)}
                  onMouseLeave={() => setHoveredIdx(null)}
                />
              );
            })}

            {/* Custom per-slice color CSS injection for theme support */}
            <style>
              {slices
                .map(
                  (s) =>
                    `path[data-category="${s.cat.category_name}"] { --slice-color: ${s.color}; }
                     .dark path[data-category="${s.cat.category_name}"] { --slice-color: ${s.darkColor}; }`
                )
                .join('\n')}
            </style>
          </svg>

          {/* Center Callout */}
          <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none text-center">
            {activeSlice ? (
              <>
                <span className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 truncate max-w-[90px]">
                  {activeSlice.cat.category_name}
                </span>
                <span className="text-sm font-semibold font-mono text-neutral-900 dark:text-neutral-100 tabular-nums">
                  {activeSlice.cat.percentage.toFixed(1)}%
                </span>
              </>
            ) : (
              <>
                <span className="text-[10px] font-mono uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
                  Total Spent
                </span>
                <span className="text-sm font-semibold font-mono text-neutral-900 dark:text-neutral-100 tabular-nums">
                  {formatMinor(totalExpenseMinor, exponent)}
                </span>
                <span className="text-[10px] font-mono text-neutral-400">{currency}</span>
              </>
            )}
          </div>
        </div>

        {/* Sourced Evidence Table */}
        <div className="md:col-span-7 flex flex-col space-y-1.5 overflow-hidden">
          <div className="overflow-y-auto max-h-52 divide-y divide-neutral-200 dark:divide-neutral-800 pr-1">
            {slices.map((s) => {
              const isHovered = hoveredIdx === s.index;
              return (
                <div
                  key={s.cat.category_name}
                  onMouseEnter={() => setHoveredIdx(s.index)}
                  onMouseLeave={() => setHoveredIdx(null)}
                  className={`py-2 px-2 flex items-center justify-between gap-3 text-xs transition-colors rounded cursor-pointer ${
                    isHovered
                      ? 'bg-neutral-100 dark:bg-neutral-900 text-neutral-900 dark:text-neutral-100'
                      : 'hover:bg-neutral-100/60 dark:hover:bg-neutral-900/40'
                  }`}
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      className="w-2.5 h-2.5 rounded-full shrink-0"
                      style={{ backgroundColor: s.color }}
                    />
                    <span className="font-medium font-sans truncate text-neutral-900 dark:text-neutral-100">
                      {s.cat.category_name}
                    </span>
                    <span className="text-[10px] font-mono text-neutral-400">
                      ({s.cat.tx_count} {s.cat.tx_count === 1 ? 'tx' : 'txs'})
                    </span>
                  </div>

                  <div className="flex items-center gap-3 shrink-0 font-mono tabular-nums">
                    <div className="w-12 text-right text-neutral-500 dark:text-neutral-400">
                      {s.cat.percentage.toFixed(1)}%
                    </div>
                    <div className="w-20 text-right font-medium text-neutral-900 dark:text-neutral-100">
                      {formatMinor(s.cat.amount_minor, exponent)}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
};
