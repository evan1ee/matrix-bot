import React, { useState, useEffect, useRef, useMemo } from 'react';
import { Clock, Moon, Briefcase, Trash2 } from 'lucide-react';
import type { TimeAllocation, TimeCategoryId } from './types';
import { TIME_CATEGORIES } from './types';

interface LifeTimeGridProps {
  timeAllocation: TimeAllocation;
  onChange: (newAllocation: TimeAllocation) => void;
  readOnly?: boolean;
  isToday?: boolean;
}

const getBlockTimeRange = (blockIdx: number) => {
  const startMinutes = blockIdx * 5;
  const startHour = Math.floor(startMinutes / 60);
  const startMin = startMinutes % 60;
  const endMinutes = startMinutes + 5;
  const endHour = Math.floor(endMinutes / 60);
  const endMin = endMinutes % 60;

  const startStr = `${String(startHour).padStart(2, '0')}:${String(startMin).padStart(2, '0')}`;
  const endStr = endHour === 24 ? '24:00' : `${String(endHour).padStart(2, '0')}:${String(endMin).padStart(2, '0')}`;
  return {
    start: startStr,
    end: endStr,
    range: `${startStr} – ${endStr}`,
  };
};

export const LifeTimeGrid: React.FC<LifeTimeGridProps> = ({
  timeAllocation,
  onChange,
  readOnly = false,
  isToday = false,
}) => {
  const [selectedCategory, setSelectedCategory] = useState<TimeCategoryId | 'erase'>('deep_work');
  const [hoveredBlock, setHoveredBlock] = useState<{
    idx: number;
    range: string;
    cfg: (typeof TIME_CATEGORIES)[TimeCategoryId] | null;
    x: number;
    y: number;
  } | null>(null);

  const isMouseDownRef = useRef(false);
  const lastPaintedIdxRef = useRef<number | null>(null);
  const selectedCategoryRef = useRef<TimeCategoryId | 'erase'>('deep_work');
  const timelineRef = useRef<HTMLDivElement>(null);
  const hasAutoScrolledRef = useRef(false);

  selectedCategoryRef.current = selectedCategory;

  // Window pointerup listener to ensure drag always ends cleanly
  useEffect(() => {
    const handleGlobalMouseUp = () => {
      isMouseDownRef.current = false;
      lastPaintedIdxRef.current = null;
    };
    window.addEventListener('pointerup', handleGlobalMouseUp);
    window.addEventListener('pointercancel', handleGlobalMouseUp);
    window.addEventListener('blur', handleGlobalMouseUp);
    return () => {
      window.removeEventListener('pointerup', handleGlobalMouseUp);
      window.removeEventListener('pointercancel', handleGlobalMouseUp);
      window.removeEventListener('blur', handleGlobalMouseUp);
    };
  }, []);

  // Compute live current 5-minute block index (0..287) if today
  const nowBlockIdx = useMemo(() => {
    if (!isToday) return null;
    const now = new Date();
    const h = now.getHours();
    const m = now.getMinutes();
    return h * 12 + Math.floor(m / 5);
  }, [isToday]);

  // Auto-scroll timeline container to roughly center current hour on open
  useEffect(() => {
    if (isToday && nowBlockIdx !== null && !hasAutoScrolledRef.current && timelineRef.current) {
      hasAutoScrolledRef.current = true;
      const currentHour = Math.floor(nowBlockIdx / 12);
      const targetScrollTop = Math.max(0, (currentHour - 3) * 28);
      timelineRef.current.scrollTop = targetScrollTop;
    }
  }, [isToday, nowBlockIdx]);

  // Compute hours allocated per category (each block is 5 mins = 5/60 hours)
  const stats = useMemo(() => {
    const counts: Record<string, number> = {};
    let totalAssigned = 0;
    for (let i = 0; i < 288; i++) {
      const cat = timeAllocation[i];
      if (cat) {
        counts[cat] = (counts[cat] || 0) + 1;
        totalAssigned += 1;
      }
    }
    const totalHours = ((totalAssigned * 5) / 60).toFixed(1);
    const categoryStats = (Object.keys(TIME_CATEGORIES) as TimeCategoryId[])
      .map((catId) => {
        const blocks = counts[catId] || 0;
        return {
          catId,
          cfg: TIME_CATEGORIES[catId],
          blocks,
          hours: ((blocks * 5) / 60).toFixed(1),
        };
      })
      .filter((s) => s.blocks > 0);

    return { totalHours, categoryStats, totalAssigned };
  }, [timeAllocation]);

  const paintBlock = (idx: number, category: TimeCategoryId | 'erase') => {
    if (readOnly) return;
    const currentVal = timeAllocation[idx] || null;
    const newVal = category === 'erase' ? null : category;
    if (currentVal === newVal) return;

    const next = [...timeAllocation];
    while (next.length < 288) next.push(null);
    next[idx] = newVal;
    onChange(next);
  };

  const updateHoveredBlock = (e: React.PointerEvent, blockIdx: number) => {
    if (!timelineRef.current) return;
    const rect = timelineRef.current.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const catId = (timeAllocation[blockIdx] || null) as TimeCategoryId | null;
    const cfg = catId ? TIME_CATEGORIES[catId] : null;
    const { range } = getBlockTimeRange(blockIdx);
    setHoveredBlock({ idx: blockIdx, range, cfg, x, y });
  };

  const handleBlockPointerDown = (e: React.PointerEvent, idx: number) => {
    if (readOnly || e.button !== 0) return;
    isMouseDownRef.current = true;
    lastPaintedIdxRef.current = idx;
    paintBlock(idx, selectedCategoryRef.current);
    updateHoveredBlock(e, idx);
  };

  const handleBlockPointerEnter = (e: React.PointerEvent, idx: number) => {
    if (!readOnly && isMouseDownRef.current && lastPaintedIdxRef.current !== idx) {
      lastPaintedIdxRef.current = idx;
      paintBlock(idx, selectedCategoryRef.current);
    }
    updateHoveredBlock(e, idx);
  };

  // Quick preset fills
  const applyPreset = (type: 'sleep' | 'workday' | 'clear') => {
    if (readOnly) return;
    const next = [...timeAllocation];
    while (next.length < 288) next.push(null);

    if (type === 'sleep') {
      // 23:00 - 07:00 (hours 23 and 0..6: blocks 276..287 and 0..83)
      for (let i = 23 * 12; i < 288; i++) next[i] = 'sleep';
      for (let i = 0; i < 7 * 12; i++) next[i] = 'sleep';
    } else if (type === 'workday') {
      // 09:00 - 18:00 (blocks 108..215)
      for (let i = 9 * 12; i < 18 * 12; i++) next[i] = 'deep_work';
    } else if (type === 'clear') {
      for (let i = 0; i < 288; i++) next[i] = null;
    }
    onChange(next);
  };

  const renderHourRow = (hour: number) => {
    const hourStr = String(hour).padStart(2, '0') + ':00';
    const isCurrentHour = nowBlockIdx !== null && Math.floor(nowBlockIdx / 12) === hour;

    return (
      <div
        key={hour}
        className={`flex items-center gap-1.5 h-6 px-1 rounded transition-colors duration-150 ${
          isCurrentHour
            ? 'bg-blue-50/50 dark:bg-blue-950/20'
            : 'hover:bg-neutral-50/60 dark:hover:bg-neutral-800/30'
        }`}
      >
        {/* Hour Label */}
        <div className="w-8 shrink-0 flex items-center gap-0.5 text-[10px] font-mono font-medium text-neutral-500 dark:text-neutral-400 tabular-nums">
          <span>{hourStr}</span>
          {isCurrentHour && (
            <span className="w-1.5 h-1.5 rounded-full bg-blue-500" title="Current hour" />
          )}
        </div>

        {/* 12 Five-minute Blocks (:00, :05, :10, ..., :55) */}
        <div className="grid grid-cols-12 gap-0.5 sm:gap-1 flex-1">
          {Array.from({ length: 12 }).map((_, minuteIdx) => {
            const blockIdx = hour * 12 + minuteIdx;
            const catId = (timeAllocation[blockIdx] || null) as TimeCategoryId | null;
            const cfg = catId ? TIME_CATEGORIES[catId] : null;
            const isNowBlock = nowBlockIdx === blockIdx;
            const { range } = getBlockTimeRange(blockIdx);

            return (
              <div
                key={minuteIdx}
                data-block-idx={blockIdx}
                data-testid="time-block"
                onPointerDown={(e) => handleBlockPointerDown(e, blockIdx)}
                onPointerEnter={(e) => handleBlockPointerEnter(e, blockIdx)}
                onPointerMove={(e) => updateHoveredBlock(e, blockIdx)}
                onPointerLeave={() => setHoveredBlock(null)}
                onPointerUp={() => {
                  isMouseDownRef.current = false;
                  lastPaintedIdxRef.current = null;
                }}
                title={`${range} • ${cfg ? `${cfg.emoji} ${cfg.label}` : 'Unallocated'}`}
                className={`h-5 rounded-[2px] border transition-colors duration-150 select-none ${
                  readOnly ? 'cursor-default' : 'cursor-pointer'
                } ${
                  cfg
                    ? `border-transparent ${cfg.blockClass} text-white shadow-2xs hover:opacity-90 hover:ring-1 hover:ring-neutral-900/30 dark:hover:ring-white/30`
                    : 'bg-neutral-100/80 dark:bg-neutral-800/50 border-neutral-200/80 dark:border-neutral-700/60 hover:bg-neutral-200 dark:hover:bg-neutral-700 hover:border-neutral-300 dark:hover:border-neutral-600'
                } ${
                  isNowBlock
                    ? 'ring-1.5 ring-blue-500 dark:ring-blue-400 ring-offset-1 dark:ring-offset-neutral-900'
                    : ''
                }`}
              />
            );
          })}
        </div>
      </div>
    );
  };

  return (
    <div className="space-y-2 select-none">
      {/* 1. Header with Stats & Quick Presets */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <Clock className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
          <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100">
            24-Hour Time Allocation
          </span>
          <span className="text-[10px] font-mono text-neutral-500 dark:text-neutral-400 tabular-nums">
            ({stats.totalHours}h / 24h tracked)
          </span>
        </div>

        {!readOnly && (
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => applyPreset('sleep')}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer shadow-2xs"
              title="Fill 23:00 to 07:00 as Sleep"
            >
              <Moon className="w-2.5 h-2.5 text-indigo-500" />
              <span>Sleep</span>
            </button>

            <button
              type="button"
              onClick={() => applyPreset('workday')}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer shadow-2xs"
              title="Fill 09:00 to 18:00 as Deep Work"
            >
              <Briefcase className="w-2.5 h-2.5 text-neutral-600 dark:text-neutral-300" />
              <span>Workday</span>
            </button>

            <button
              type="button"
              onClick={() => applyPreset('clear')}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-rose-50 dark:hover:bg-rose-950/40 hover:border-rose-200 dark:hover:border-rose-900 text-neutral-600 dark:text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 transition-colors duration-150 cursor-pointer shadow-2xs"
              title="Clear all blocks"
            >
              <Trash2 className="w-2.5 h-2.5" />
              <span>Clear</span>
            </button>
          </div>
        )}
      </div>

      {/* 2. Palette Selector (Only when editable) */}
      {!readOnly && (
        <div className="p-1.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/70 dark:bg-neutral-800/40 space-y-1">
          <div className="flex flex-wrap gap-1">
            {(Object.keys(TIME_CATEGORIES) as TimeCategoryId[]).map((catId) => {
              const cfg = TIME_CATEGORIES[catId];
              const isSelected = selectedCategory === catId;
              return (
                <button
                  key={catId}
                  type="button"
                  onClick={() => setSelectedCategory(catId)}
                  className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[11px] font-medium border transition-colors duration-150 cursor-pointer ${
                    isSelected
                      ? `${cfg.activeClass} shadow-2xs ring-1 ring-neutral-900/20 dark:ring-neutral-100/20 font-semibold`
                      : 'border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700'
                  }`}
                >
                  <span className="text-xs">{cfg.emoji}</span>
                  <span>{cfg.label}</span>
                </button>
              );
            })}

            <button
              type="button"
              onClick={() => setSelectedCategory('erase')}
              className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[11px] font-medium border transition-colors duration-150 cursor-pointer ${
                selectedCategory === 'erase'
                  ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 border-neutral-900 dark:border-neutral-100 font-semibold shadow-2xs'
                  : 'border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700'
              }`}
            >
              <span className="text-xs">🧼</span>
              <span>Eraser</span>
            </button>
          </div>
        </div>
      )}

      {/* 3. 24-Hour Timeline Grid Container (2 Columns: AM & PM) */}
      <div
        ref={timelineRef}
        onPointerLeave={() => {
          isMouseDownRef.current = false;
          setHoveredBlock(null);
        }}
        className="relative border border-neutral-200 dark:border-neutral-800 rounded-lg p-2 bg-white dark:bg-neutral-900 space-y-1"
      >
        {/* Floating Tooltip HUD */}
        {hoveredBlock && (
          <div
            data-testid="time-block-hud"
            className="pointer-events-none absolute z-30 px-2.5 py-1 rounded-md text-xs font-mono shadow-xs bg-neutral-900 text-neutral-100 dark:bg-neutral-100 dark:text-neutral-900 border border-neutral-800 dark:border-neutral-200 whitespace-nowrap will-change-transform flex items-center gap-1.5"
            style={{
              left: `${Math.min(hoveredBlock.x + 10, (timelineRef.current?.clientWidth || 320) - 170)}px`,
              top: `${Math.max(6, hoveredBlock.y - 32)}px`,
            }}
          >
            <span className="font-mono font-semibold tabular-nums tracking-tight">
              {hoveredBlock.range}
            </span>
            <span className="text-neutral-400 dark:text-neutral-500">•</span>
            {hoveredBlock.cfg ? (
              <span className="font-sans text-[11px] font-medium flex items-center gap-1">
                <span>{hoveredBlock.cfg.emoji}</span>
                <span>{hoveredBlock.cfg.label}</span>
              </span>
            ) : (
              <span className="font-sans text-[11px] text-neutral-400 dark:text-neutral-500">
                Unallocated
              </span>
            )}
          </div>
        )}

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-2.5 gap-y-0.5">
          {/* Sub-column 1: 00:00 - 12:00 */}
          <div className="space-y-0.5">
            <div className="text-[9px] font-mono font-medium text-neutral-400 dark:text-neutral-500 uppercase tracking-wider pb-0.5 border-b border-neutral-100 dark:border-neutral-800 flex justify-between px-1">
              <span>00:00 – 12:00</span>
              <span>Morning</span>
            </div>
            {Array.from({ length: 12 }).map((_, i) => renderHourRow(i))}
          </div>

          {/* Sub-column 2: 12:00 - 24:00 */}
          <div className="space-y-0.5">
            <div className="text-[9px] font-mono font-medium text-neutral-400 dark:text-neutral-500 uppercase tracking-wider pb-0.5 border-b border-neutral-100 dark:border-neutral-800 flex justify-between px-1">
              <span>12:00 – 24:00</span>
              <span>Evening</span>
            </div>
            {Array.from({ length: 12 }).map((_, i) => renderHourRow(i + 12))}
          </div>
        </div>
      </div>

      {/* 4. Category Breakdown Pill Badges (Positioned below grid to maintain layout stability) */}
      {stats.categoryStats.length > 0 && (
        <div className="flex flex-wrap gap-1.5 pt-0.5">
          {stats.categoryStats.map((s) => (
            <span
              key={s.catId}
              className={`inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono border ${s.cfg.badgeClass}`}
            >
              <span>{s.cfg.emoji}</span>
              <span className="font-sans font-medium">{s.cfg.label}:</span>
              <span className="font-bold tabular-nums">{s.hours}h</span>
            </span>
          ))}
        </div>
      )}
    </div>
  );
};
