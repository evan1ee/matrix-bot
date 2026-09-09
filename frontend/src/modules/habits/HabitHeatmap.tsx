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

  // Generate 52 weeks (364 days) ending on today or current date
  const { weeks, monthLabels } = useMemo(() => {
    const now = today ? new Date(today + 'T00:00:00') : new Date();
    // End date is today
    const endDate = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    
    // Find previous Sunday to start week alignment or Monday
    // We display 53 columns (approx 1 year)
    const totalDays = 52 * 7;
    const startDate = new Date(endDate);
    startDate.setDate(endDate.getDate() - totalDays + (6 - endDate.getDay()));

    const weeksArr: { date: string; count: number; dayOfWeek: number; month: number }[][] = [];
    const months: { name: string; weekIndex: number }[] = [];
    let currentWeek: { date: string; count: number; dayOfWeek: number; month: number }[] = [];
    let lastMonth = -1;

    const cur = new Date(startDate);
    let weekIdx = 0;

    while (cur <= endDate) {
      const y = cur.getFullYear();
      const m = String(cur.getMonth() + 1).padStart(2, '0');
      const d = String(cur.getDate()).padStart(2, '0');
      const dateStr = `${y}-${m}-${d}`;
      const count = dayMap.get(dateStr) || 0;
      const dayOfWeek = cur.getDay(); // 0 = Sun, 6 = Sat
      const month = cur.getMonth();

      if (month !== lastMonth && currentWeek.length === 0) {
        months.push({
          name: cur.toLocaleString('en-US', { month: 'short' }),
          weekIndex: weekIdx,
        });
        lastMonth = month;
      }

      currentWeek.push({
        date: dateStr,
        count,
        dayOfWeek,
        month,
      });

      if (dayOfWeek === 6 || cur.getTime() === endDate.getTime()) {
        weeksArr.push(currentWeek);
        currentWeek = [];
        weekIdx++;
      }

      cur.setDate(cur.getDate() + 1);
    }

    if (currentWeek.length > 0) {
      weeksArr.push(currentWeek);
    }

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
      {/* Month Labels */}
      <div className="flex text-[10px] font-mono text-neutral-400 mb-1.5 pl-6 min-w-[720px]">
        {weeks.map((_, i) => {
          const m = monthLabels.find((ml) => ml.weekIndex === i);
          return (
            <div key={i} className="w-3 mr-1 text-center shrink-0">
              {m ? m.name : ''}
            </div>
          );
        })}
      </div>

      <div className="flex items-start gap-2 min-w-[720px]">
        {/* Day of Week Labels */}
        <div className="flex flex-col gap-1 text-[9px] font-mono text-neutral-400 shrink-0 select-none pt-0.5">
          <span className="h-2.5 leading-none"></span>
          <span className="h-2.5 leading-none">Mon</span>
          <span className="h-2.5 leading-none"></span>
          <span className="h-2.5 leading-none">Wed</span>
          <span className="h-2.5 leading-none"></span>
          <span className="h-2.5 leading-none">Fri</span>
          <span className="h-2.5 leading-none"></span>
        </div>

        {/* Heatmap Grid */}
        <div className="flex gap-1">
          {weeks.map((week, wIdx) => (
            <div key={wIdx} className="flex flex-col gap-1">
              {/* Ensure 7 slots per week */}
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
                        x: rect.left + window.scrollX,
                        y: rect.top + window.scrollY - 30,
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

      {/* Legend & Stats Footer */}
      <div className="flex items-center justify-between mt-3 text-[11px] font-mono text-neutral-500 min-w-[720px]">
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
          className="fixed z-50 px-2 py-1 text-xs font-mono rounded bg-neutral-900 text-white dark:bg-neutral-100 dark:text-black shadow-md pointer-events-none transform -translate-x-1/2"
          style={{ left: hoveredDay.x + 5, top: hoveredDay.y }}
        >
          {hoveredDay.count} {hoveredDay.count === 1 ? 'habit' : 'habits'} on {hoveredDay.date}
        </div>
      )}
    </div>
  );
};
