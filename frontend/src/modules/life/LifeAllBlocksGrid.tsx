import React, { useMemo, useState, useRef } from 'react';
import type { LifeHorizonDay, MoodType } from './types';
import { MOOD_CONFIG } from './types';

interface LifeAllBlocksGridProps {
  days: LifeHorizonDay[];
  onSelectDate: (date: string) => void;
  selectedDate: string | null;
}

interface HoverInfo {
  date: string;
  year: number;
  status: string;
  mood: string | null;
  x: number;
  y: number;
}

export const LifeAllBlocksGrid: React.FC<LifeAllBlocksGridProps> = ({
  days,
  onSelectDate,
  selectedDate,
}) => {
  const [hoverInfo, setHoverInfo] = useState<HoverInfo | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const todayStr = useMemo(() => new Date().toISOString().split('T')[0], []);
  const currentActualYear = new Date().getFullYear();

  // Group days by year
  const yearsGrouped = useMemo(() => {
    const map = new Map<number, LifeHorizonDay[]>();
    for (const d of days) {
      const list = map.get(d.year) || [];
      list.push(d);
      map.set(d.year, list);
    }
    return Array.from(map.entries()).sort((a, b) => a[0] - b[0]);
  }, [days]);

  // Extract distinct decades for the jump bar
  const decades = useMemo(() => {
    const set = new Set<number>();
    for (const [yr] of yearsGrouped) {
      set.add(Math.floor(yr / 10) * 10);
    }
    return Array.from(set).sort((a, b) => a - b);
  }, [yearsGrouped]);

  const scrollToDecade = (decade: number) => {
    const el = document.getElementById(`decade-${decade}`);
    if (el) {
      el.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }
  };

  const scrollToToday = () => {
    const el = document.getElementById(`day-${todayStr}`) || document.getElementById(`year-${currentActualYear}`);
    if (el) {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
  };

  return (
    <div
      ref={containerRef}
      className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-5 space-y-5 relative"
    >
      {/* Header & Controls */}
      <div className="flex flex-col md:flex-row md:items-baseline justify-between gap-3 pb-3 border-b border-zinc-100 dark:border-zinc-800">
        <div>
          <h2 className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
            Lifespan day grid
          </h2>
          <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">
            {days.length.toLocaleString()} total days. White is future, green is today, and colors mark recorded moods.
          </p>
        </div>

        {/* Legend */}
        <div className="flex flex-wrap items-center gap-3 text-xs text-zinc-500 dark:text-zinc-400">
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-zinc-200 dark:bg-zinc-800" />
            <span>Past</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-emerald-500 ring-1 ring-emerald-400" />
            <span>Today</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-amber-400 dark:bg-amber-500" />
            <span>Amazing</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-emerald-500" />
            <span>Good</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-cyan-500" />
            <span>Neutral</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-purple-500" />
            <span>Low</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-rose-500" />
            <span>Rough</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-white dark:bg-zinc-950 border border-zinc-200 dark:border-zinc-800" />
            <span>Future</span>
          </div>
        </div>
      </div>

      {/* Decade Quick-Jump Bar & Jump to Today */}
      <div className="flex items-center justify-between gap-2 overflow-x-auto pb-1 text-xs">
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-xs text-zinc-400 dark:text-zinc-500 mr-1">
            Jump to:
          </span>
          {decades.map((dec) => (
            <button
              key={dec}
              type="button"
              onClick={() => scrollToDecade(dec)}
              className="px-2 py-0.5 rounded text-xs font-mono border border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/60 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
            >
              {dec}s
            </button>
          ))}
        </div>

        <button
          type="button"
          onClick={scrollToToday}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-xs font-medium border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 hover:bg-zinc-100 dark:hover:bg-zinc-700 text-zinc-800 dark:text-zinc-200 transition-colors shrink-0"
        >
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
          <span>Jump to today</span>
        </button>
      </div>

      {/* Unified Grid: Year by Year */}
      <div className="space-y-1.5">
        {yearsGrouped.map(([yr, yearDays]) => {
          const isDecadeStart = yr % 10 === 0;
          const isCurrentYear = yr === currentActualYear;
          const decadeVal = Math.floor(yr / 10) * 10;

          return (
            <div key={yr} id={isDecadeStart ? `decade-${decadeVal}` : undefined} className="space-y-1">
              {/* Decade Header Anchor */}
              {isDecadeStart && (
                <div className="pt-3 pb-1 text-xs font-medium text-zinc-400 dark:text-zinc-500 flex items-center gap-2">
                  <span>{decadeVal}s</span>
                  <span className="flex-1 h-px bg-zinc-100 dark:bg-zinc-800" />
                </div>
              )}

              {/* Year Row */}
              <div
                id={`year-${yr}`}
                className={`flex items-center gap-2 px-1.5 py-1 rounded transition-colors group ${
                  isCurrentYear
                    ? 'bg-zinc-100/70 dark:bg-zinc-800/50'
                    : 'hover:bg-zinc-50 dark:hover:bg-zinc-800/30'
                }`}
              >
                {/* Year Label - NO AGE */}
                <div className="w-11 shrink-0 flex items-center gap-1">
                  <span
                    className={`font-mono text-xs tabular-nums ${
                      isCurrentYear
                        ? 'font-semibold text-zinc-900 dark:text-zinc-100'
                        : 'text-zinc-400 dark:text-zinc-500 group-hover:text-zinc-700 dark:group-hover:text-zinc-300'
                    }`}
                  >
                    {yr}
                  </span>
                  {isCurrentYear && (
                    <span className="w-1 h-1 rounded-full bg-emerald-500" />
                  )}
                </div>

                {/* Day Blocks Row */}
                <div className="flex-1 flex items-center gap-[1px] sm:gap-[1.5px] overflow-hidden h-3.5 sm:h-4">
                  {yearDays.map((d) => {
                    const isToday = d.date === todayStr;
                    const isSelected = d.date === selectedDate;
                    const moodKey = (d.mood?.toLowerCase() || '') as MoodType;
                    const moodCfg = MOOD_CONFIG[moodKey];

                    let blockClass = '';
                    if (isToday) {
                      blockClass =
                        'bg-emerald-500 ring-1 ring-emerald-400 z-10';
                    } else if (moodCfg) {
                      blockClass = moodCfg.bgClass;
                    } else if (d.status === 'past_empty') {
                      blockClass =
                        'bg-zinc-200 dark:bg-zinc-800 hover:bg-zinc-300 dark:hover:bg-zinc-700';
                    } else {
                      // Future
                      blockClass =
                        'bg-white dark:bg-zinc-950 border border-zinc-200 dark:border-zinc-800 hover:border-zinc-400 dark:hover:border-zinc-600';
                    }

                    return (
                      <button
                        key={d.date}
                        id={`day-${d.date}`}
                        type="button"
                        data-date={d.date}
                        onClick={() => onSelectDate(d.date)}
                        onMouseEnter={(e) => {
                          const rect = e.currentTarget.getBoundingClientRect();
                          const parentRect = containerRef.current?.getBoundingClientRect();
                          if (parentRect) {
                            setHoverInfo({
                              date: d.date,
                              year: d.year,
                              status: d.status,
                              mood: d.mood,
                              x: rect.left - parentRect.left + rect.width / 2,
                              y: rect.top - parentRect.top,
                            });
                          }
                        }}
                        onMouseLeave={() => setHoverInfo(null)}
                        className={`h-full flex-1 min-w-[2px] max-w-[5px] rounded-[1px] cursor-pointer transition-opacity ${blockClass} ${
                          isSelected ? 'ring-1 ring-zinc-900 dark:ring-zinc-100 z-20' : 'hover:opacity-80'
                        }`}
                        aria-label={d.date}
                      />
                    );
                  })}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Floating Hover Tooltip */}
      {hoverInfo && (
        <div
          className="pointer-events-none absolute z-30 px-2.5 py-1 rounded text-xs font-medium bg-zinc-900 text-zinc-100 dark:bg-zinc-100 dark:text-zinc-900 border border-zinc-800 dark:border-zinc-200 whitespace-nowrap transform -translate-x-1/2 -translate-y-full -mt-2 shadow-sm"
          style={{ left: hoverInfo.x, top: hoverInfo.y }}
        >
          <div className="flex items-center gap-1.5">
            <span className="font-mono">{hoverInfo.date}</span>
            {hoverInfo.date === todayStr && (
              <span className="text-[10px] text-emerald-400 dark:text-emerald-600 font-medium">
                (Today)
              </span>
            )}
            {hoverInfo.mood && (
              <span className="capitalize text-zinc-300 dark:text-zinc-600 font-normal">
                • {hoverInfo.mood}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
