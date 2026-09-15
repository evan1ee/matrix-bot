import React, { useMemo, useState } from 'react';
import type { HeatmapDay } from './types';

interface HabitHeatmapProps {
  days: HeatmapDay[];
  today?: string;
}

export const HabitHeatmap: React.FC<HabitHeatmapProps> = ({ days, today }) => {
  const [hoveredDay, setHoveredDay] = useState<{ date: string; count: number; x: number; y: number } | null>(null);

  const dayMap = useMemo(() => {
    const map = new Map<string, number>();
    for (const d of days) {
      map.set(d.date, d.count);
    }
    return map;
  }, [days]);

  // Generate 52 weeks (approx 1 year) ending on today or specified current date
  const { weeks, monthLabels } = useMemo(() => {
    const now = today ? new Date(today + 'T00:00:00') : new Date();
    // End date is today
    const endDate = new Date(now.getFullYear(), now.getMonth(), now.getDate());

    // The current week's Sunday (0 = Sun, 1 = Mon, ..., 6 = Sat)
    const endDayOfWeek = endDate.getDay();
    const thisWeekSunday = new Date(endDate);
    thisWeekSunday.setDate(endDate.getDate() - endDayOfWeek);

    // 52 weeks total (51 full weeks before this week + this week)
    const startDate = new Date(thisWeekSunday);
    startDate.setDate(thisWeekSunday.getDate() - 51 * 7);

    const weeksArr: { date: string; count: number; dayOfWeek: number; month: number; dayOfMonth: number }[][] = [];
    let currentWeek: { date: string; count: number; dayOfWeek: number; month: number; dayOfMonth: number }[] = [];

    const cur = new Date(startDate);

    while (cur <= endDate) {
      const y = cur.getFullYear();
      const m = String(cur.getMonth() + 1).padStart(2, '0');
      const d = String(cur.getDate()).padStart(2, '0');
      const dateStr = `${y}-${m}-${d}`;
      const count = dayMap.get(dateStr) || 0;
      const dayOfWeek = cur.getDay(); // 0 = Sun, 6 = Sat
      const month = cur.getMonth();
      const dayOfMonth = cur.getDate();

      currentWeek.push({
        date: dateStr,
        count,
        dayOfWeek,
        month,
        dayOfMonth,
      });

      if (dayOfWeek === 6 || cur.getTime() === endDate.getTime()) {
        weeksArr.push(currentWeek);
        currentWeek = [];
      }

      cur.setDate(cur.getDate() + 1);
    }

    if (currentWeek.length > 0) {
      weeksArr.push(currentWeek);
    }

    // Determine month label positions based on where each month begins
    const months: { name: string; weekIndex: number }[] = [];
    const firstMonthStart = weeksArr.findIndex((w) => w.some((d) => d.dayOfMonth === 1));
    if (firstMonthStart >= 2 && weeksArr.length > 0 && weeksArr[0].length > 0) {
      const d0 = new Date(weeksArr[0][0].date + 'T00:00:00');
      months.push({
        name: d0.toLocaleString('en-US', { month: 'short' }),
        weekIndex: 0,
      });
    }

    weeksArr.forEach((w, wIdx) => {
      const firstOfMonth = w.find((d) => d.dayOfMonth === 1);
      if (firstOfMonth) {
        const d = new Date(firstOfMonth.date + 'T00:00:00');
        months.push({
          name: d.toLocaleString('en-US', { month: 'short' }),
          weekIndex: wIdx,
        });
      }
    });

    return { weeks: weeksArr, monthLabels: months };
  }, [dayMap, today]);

  const getColorClass = (count: number) => {
    if (count === 0) return 'bg-neutral-200 dark:bg-neutral-800';
    if (count === 1) return 'bg-neutral-400 dark:bg-neutral-600';
    if (count === 2) return 'bg-neutral-600 dark:bg-neutral-400';
    return 'bg-neutral-900 dark:bg-neutral-100';
  };

  return (
    <div className="relative w-full overflow-x-auto select-none pt-1">
      <div className="flex items-start gap-2 min-w-[760px]">
        {/* Day of Week Labels */}
        <div className="flex flex-col gap-1 text-[9px] font-mono text-neutral-400 dark:text-neutral-500 shrink-0 select-none pt-5">
          <span className="h-2.5 leading-none" />
          <span className="h-2.5 leading-none">Mon</span>
          <span className="h-2.5 leading-none" />
          <span className="h-2.5 leading-none">Wed</span>
          <span className="h-2.5 leading-none" />
          <span className="h-2.5 leading-none">Fri</span>
          <span className="h-2.5 leading-none" />
        </div>

        {/* Heatmap Column: Month labels on top, grid below */}
        <div className="flex flex-col select-none">
          {/* Month Labels Track: strictly aligned with week columns (each column is 14px = 10px cell + 4px gap) */}
          <div className="relative h-4 mb-1" style={{ width: `${weeks.length * 14}px` }}>
            {monthLabels.map((m) => (
              <span
                key={`${m.name}-${m.weekIndex}`}
                className="absolute text-[10px] font-mono text-neutral-400 dark:text-neutral-500 leading-none select-none"
                style={{ left: `${m.weekIndex * 14}px` }}
              >
                {m.name}
              </span>
            ))}
          </div>

          {/* Heatmap Grid */}
          <div className="flex gap-1">
            {weeks.map((week, wIdx) => (
              <div key={wIdx} className="flex flex-col gap-1">
                {/* Ensure 7 slots per week: 0=Sun, 1=Mon, ..., 6=Sat */}
                {Array.from({ length: 7 }).map((_, dIdx) => {
                  const day = week.find((d) => d.dayOfWeek === dIdx);
                  if (!day) {
                    return <div key={dIdx} className="w-2.5 h-2.5" />;
                  }
                  return (
                    <div
                      key={day.date}
                      data-date={day.date}
                      data-count={day.count}
                      className={`w-2.5 h-2.5 rounded-xs transition-colors cursor-pointer ${getColorClass(day.count)} hover:ring-1 hover:ring-neutral-400 dark:hover:ring-neutral-200`}
                      onMouseEnter={(e) => {
                        const rect = e.currentTarget.getBoundingClientRect();
                        setHoveredDay({
                          date: day.date,
                          count: day.count,
                          x: rect.left + rect.width / 2,
                          y: rect.top - 6,
                        });
                      }}
                      onMouseLeave={() => setHoveredDay(null)}
                    />
                  );
                })}
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Legend & Stats Footer */}
      <div className="flex items-center justify-between mt-3 text-[11px] font-mono text-neutral-500 dark:text-neutral-400 min-w-[760px]">
        <div className="flex items-center gap-1.5">
          <span>Less</span>
          <div className="w-2.5 h-2.5 rounded-xs bg-neutral-200 dark:bg-neutral-800" />
          <div className="w-2.5 h-2.5 rounded-xs bg-neutral-400 dark:bg-neutral-600" />
          <div className="w-2.5 h-2.5 rounded-xs bg-neutral-600 dark:bg-neutral-400" />
          <div className="w-2.5 h-2.5 rounded-xs bg-neutral-900 dark:bg-neutral-100" />
          <span>More</span>
        </div>
        <span>52 weeks contribution</span>
      </div>

      {/* Floating Tooltip */}
      {hoveredDay && (
        <div
          className="fixed z-50 px-2 py-1 text-xs font-mono rounded bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 shadow-md pointer-events-none transform -translate-x-1/2 -translate-y-full"
          style={{ left: hoveredDay.x, top: hoveredDay.y }}
        >
          {hoveredDay.count} {hoveredDay.count === 1 ? 'habit' : 'habits'} on {hoveredDay.date}
        </div>
      )}
    </div>
  );
};
