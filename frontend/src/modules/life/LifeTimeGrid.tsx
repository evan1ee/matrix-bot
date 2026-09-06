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

export const LifeTimeGrid: React.FC<LifeTimeGridProps> = ({
  timeAllocation,
  onChange,
  readOnly = false,
  isToday = false,
}) => {
  const [selectedCategory, setSelectedCategory] = useState<TimeCategoryId | 'erase'>('deep_work');
  const isMouseDownRef = useRef(false);
  const selectedCategoryRef = useRef<TimeCategoryId | 'erase'>('deep_work');

  selectedCategoryRef.current = selectedCategory;

  // Window pointerup listener to ensure drag always ends cleanly
  useEffect(() => {
    const handleGlobalMouseUp = () => {
      isMouseDownRef.current = false;
    };
    window.addEventListener('pointerup', handleGlobalMouseUp);
    return () => window.removeEventListener('pointerup', handleGlobalMouseUp);
  }, []);

  // Compute live current 15-minute block index (0..95) if today
  const nowBlockIdx = useMemo(() => {
    if (!isToday) return null;
    const now = new Date();
    const h = now.getHours();
    const m = now.getMinutes();
    return h * 4 + Math.floor(m / 15);
  }, [isToday]);

  // Compute hours allocated per category
  const stats = useMemo(() => {
    const counts: Record<string, number> = {};
    let totalAssigned = 0;
    for (let i = 0; i < 96; i++) {
      const cat = timeAllocation[i];
      if (cat) {
        counts[cat] = (counts[cat] || 0) + 1;
        totalAssigned += 1;
      }
    }
    const totalHours = (totalAssigned * 0.25).toFixed(1);
    const categoryStats = (Object.keys(TIME_CATEGORIES) as TimeCategoryId[])
      .map((catId) => {
        const blocks = counts[catId] || 0;
        return {
          catId,
          cfg: TIME_CATEGORIES[catId],
          blocks,
          hours: (blocks * 0.25).toFixed(1),
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
    while (next.length < 96) next.push(null);
    next[idx] = newVal;
    onChange(next);
  };

  const handleBlockPointerDown = (idx: number) => {
    if (readOnly) return;
    isMouseDownRef.current = true;
    paintBlock(idx, selectedCategoryRef.current);
  };

  const handleBlockPointerEnter = (idx: number) => {
    if (readOnly || !isMouseDownRef.current) return;
    paintBlock(idx, selectedCategoryRef.current);
  };

  // Quick preset fills
  const applyPreset = (type: 'sleep' | 'workday' | 'clear') => {
    if (readOnly) return;
    const next = [...timeAllocation];
    while (next.length < 96) next.push(null);

    if (type === 'sleep') {
      // 23:00 - 07:00 (blocks 92..95 and 0..27)
      for (let i = 92; i < 96; i++) next[i] = 'sleep';
      for (let i = 0; i < 28; i++) next[i] = 'sleep';
    } else if (type === 'workday') {
      // 09:00 - 18:00 (blocks 36..71)
      for (let i = 36; i < 72; i++) next[i] = 'deep_work';
    } else if (type === 'clear') {
      for (let i = 0; i < 96; i++) next[i] = null;
    }
    onChange(next);
  };

  return (
    <div className="space-y-3 select-none">
      {/* 1. Header with Stats & Quick Presets */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Clock className="w-3.5 h-3.5 text-zinc-500 dark:text-zinc-400" />
          <span className="text-xs font-semibold text-zinc-900 dark:text-zinc-100">
            24-Hour Time Allocation
          </span>
          <span className="text-[11px] font-mono text-zinc-500 dark:text-zinc-400 tabular-nums">
            ({stats.totalHours}h / 24h tracked)
          </span>
        </div>

        {!readOnly && (
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={() => applyPreset('sleep')}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium border border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/60 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-700 dark:text-zinc-300 transition-colors cursor-pointer"
              title="Fill 23:00 to 07:00 as Sleep"
            >
              <Moon className="w-3 h-3 text-indigo-500" />
              <span>Sleep</span>
            </button>

            <button
              type="button"
              onClick={() => applyPreset('workday')}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium border border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/60 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-700 dark:text-zinc-300 transition-colors cursor-pointer"
              title="Fill 09:00 to 18:00 as Deep Work"
            >
              <Briefcase className="w-3 h-3 text-zinc-600 dark:text-zinc-300" />
              <span>Workday</span>
            </button>

            <button
              type="button"
              onClick={() => applyPreset('clear')}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium border border-zinc-200 dark:border-zinc-800 hover:bg-rose-50 dark:hover:bg-rose-950/40 text-zinc-500 dark:text-zinc-400 hover:text-rose-600 dark:hover:text-rose-400 transition-colors cursor-pointer"
              title="Clear all blocks"
            >
              <Trash2 className="w-3 h-3" />
              <span>Clear</span>
            </button>
          </div>
        )}
      </div>

      {/* 2. Palette Selector (Only when editable) */}
      {!readOnly && (
        <div className="p-2 rounded-lg border border-zinc-200 dark:border-zinc-800 bg-zinc-50/70 dark:bg-zinc-800/40 space-y-1.5">
          <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 dark:text-zinc-400">
            Paint Palette (Click or Drag across blocks)
          </div>
          <div className="flex flex-wrap gap-1.5">
            {(Object.keys(TIME_CATEGORIES) as TimeCategoryId[]).map((catId) => {
              const cfg = TIME_CATEGORIES[catId];
              const isSelected = selectedCategory === catId;
              return (
                <button
                  key={catId}
                  type="button"
                  onClick={() => setSelectedCategory(catId)}
                  className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-xs font-medium border transition-all cursor-pointer ${
                    isSelected
                      ? `${cfg.activeClass} shadow-xs ring-1 ring-zinc-900/20 dark:ring-zinc-100/20 font-semibold`
                      : 'border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                  }`}
                >
                  <span>{cfg.emoji}</span>
                  <span>{cfg.label}</span>
                </button>
              );
            })}

            <button
              type="button"
              onClick={() => setSelectedCategory('erase')}
              className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-xs font-medium border transition-all cursor-pointer ${
                selectedCategory === 'erase'
                  ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 border-zinc-900 dark:border-zinc-100 font-semibold'
                  : 'border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800'
              }`}
            >
              <span>🧼</span>
              <span>Eraser</span>
            </button>
          </div>
        </div>
      )}

      {/* 3. Category Breakdown Pill Badges */}
      {stats.categoryStats.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
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

      {/* 4. 24-Hour Timeline Grid Container (Scrollable) */}
      <div className="max-h-64 overflow-y-auto pr-1 border border-zinc-200 dark:border-zinc-800 rounded-lg p-2.5 bg-white dark:bg-zinc-900 space-y-1">
        {Array.from({ length: 24 }).map((_, hour) => {
          const hourStr = String(hour).padStart(2, '0') + ':00';
          const isCurrentHour = nowBlockIdx !== null && Math.floor(nowBlockIdx / 4) === hour;

          return (
            <div
              key={hour}
              className={`flex items-center gap-2 py-0.5 px-1 rounded transition-colors ${
                isCurrentHour ? 'bg-blue-50/50 dark:bg-blue-950/20' : ''
              }`}
            >
              {/* Hour Label */}
              <div className="w-11 shrink-0 flex items-center gap-1 text-[11px] font-mono font-medium text-zinc-500 dark:text-zinc-400 tabular-nums">
                <span>{hourStr}</span>
                {isCurrentHour && (
                  <span className="w-1.5 h-1.5 rounded-full bg-blue-500 animate-pulse" title="Current hour" />
                )}
              </div>

              {/* 4 Quarter Blocks (:00, :15, :30, :45) */}
              <div className="grid grid-cols-4 gap-1 flex-1">
                {[0, 1, 2, 3].map((quarter) => {
                  const blockIdx = hour * 4 + quarter;
                  const catId = (timeAllocation[blockIdx] || null) as TimeCategoryId | null;
                  const cfg = catId ? TIME_CATEGORIES[catId] : null;
                  const isNowBlock = nowBlockIdx === blockIdx;
                  const timeLabel = `${String(hour).padStart(2, '0')}:${String(quarter * 15).padStart(2, '0')}`;

                  return (
                    <div
                      key={quarter}
                      data-block-idx={blockIdx}
                      data-testid="time-block"
                      onPointerDown={() => handleBlockPointerDown(blockIdx)}
                      onPointerEnter={() => handleBlockPointerEnter(blockIdx)}
                      title={`${timeLabel} - ${cfg ? `${cfg.label} (${cfg.emoji})` : 'Unallocated'}`}
                      className={`h-5 rounded border transition-all flex items-center justify-center text-[10px] select-none ${
                        readOnly ? 'cursor-default' : 'cursor-pointer'
                      } ${
                        cfg
                          ? `${cfg.blockClass} text-white shadow-xs`
                          : 'bg-zinc-100/70 dark:bg-zinc-800/40 border-zinc-200/80 dark:border-zinc-700/60 hover:bg-zinc-200/80 dark:hover:bg-zinc-700/80'
                      } ${
                        isNowBlock
                          ? 'ring-2 ring-blue-500 dark:ring-blue-400 ring-offset-1 dark:ring-offset-zinc-900'
                          : ''
                      }`}
                    >
                      {cfg ? (
                        <span className="scale-90 select-none pointer-events-none">{cfg.emoji}</span>
                      ) : (
                        <span className="text-[9px] text-zinc-400/80 dark:text-zinc-600 tabular-nums select-none pointer-events-none">
                          :{String(quarter * 15).padStart(2, '0')}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};
