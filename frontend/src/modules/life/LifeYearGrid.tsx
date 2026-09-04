import React, { useMemo } from 'react';
import { ChevronLeft, ChevronRight, Calendar as CalendarIcon, Sparkles } from 'lucide-react';
import type { LifeDayItem } from './types';
import { LifeDayBlock } from './LifeDayBlock';

interface LifeYearGridProps {
  year: number;
  birthDate: string | null;
  days: LifeDayItem[];
  selectedDate: string | null;
  onSelectYear: (year: number) => void;
  onSelectDay: (day: LifeDayItem) => void;
}

const MONTH_NAMES = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];

const WEEKDAYS = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa'];

export const LifeYearGrid: React.FC<LifeYearGridProps> = ({
  year,
  birthDate,
  days,
  selectedDate,
  onSelectYear,
  onSelectDay,
}) => {
  const currentActualYear = new Date().getFullYear();
  const isCurrentYear = year === currentActualYear;

  // Compute age during this year
  const userAgeInYear = useMemo(() => {
    if (!birthDate) return null;
    const birthYear = parseInt(birthDate.split('-')[0], 10);
    if (isNaN(birthYear)) return null;
    const age = year - birthYear;
    return age >= 0 ? age : null;
  }, [birthDate, year]);

  // Map days by date string for fast lookup
  const daysMap = useMemo(() => {
    const map = new Map<string, LifeDayItem>();
    for (const d of days) {
      map.set(d.date, d);
    }
    return map;
  }, [days]);

  // Group months: 0 to 11
  const monthsData = useMemo(() => {
    return MONTH_NAMES.map((monthName, monthIndex) => {
      // First day of month
      const firstDay = new Date(Date.UTC(year, monthIndex, 1));
      const startingDayOfWeek = firstDay.getUTCDay(); // 0 = Sunday, 6 = Saturday

      // Days in this month
      const daysInMonth = new Date(Date.UTC(year, monthIndex + 1, 0)).getUTCDate();

      const monthDays: { dayNum: number; item: LifeDayItem }[] = [];
      for (let d = 1; d <= daysInMonth; d++) {
        const mm = String(monthIndex + 1).padStart(2, '0');
        const dd = String(d).padStart(2, '0');
        const dateStr = `${year}-${mm}-${dd}`;
        const item = daysMap.get(dateStr) || {
          date: dateStr,
          day_type: 'future',
        };
        monthDays.push({ dayNum: d, item });
      }

      return {
        monthName,
        startingDayOfWeek,
        monthDays,
      };
    });
  }, [year, daysMap]);

  return (
    <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800/80 rounded-xl p-4 sm:p-6 shadow-sm">
      {/* Year Navigation Bar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-6 border-b border-zinc-100 dark:border-zinc-800/80">
        <div className="flex items-center gap-3">
          <div className="flex items-center rounded-lg border border-zinc-200 dark:border-zinc-700 overflow-hidden shadow-xs bg-zinc-50 dark:bg-zinc-800">
            <button
              onClick={() => onSelectYear(year - 1)}
              title="Previous Year"
              className="p-2 hover:bg-zinc-100 dark:hover:bg-zinc-700 text-zinc-600 dark:text-zinc-300 transition-colors"
            >
              <ChevronLeft className="w-4 h-4" />
            </button>
            <span className="px-4 text-lg font-bold font-mono tabular-nums text-zinc-900 dark:text-zinc-100 min-w-[70px] text-center">
              {year}
            </span>
            <button
              onClick={() => onSelectYear(year + 1)}
              title="Next Year"
              className="p-2 hover:bg-zinc-100 dark:hover:bg-zinc-700 text-zinc-600 dark:text-zinc-300 transition-colors"
            >
              <ChevronRight className="w-4 h-4" />
            </button>
          </div>

          {userAgeInYear !== null && (
            <span className="px-2.5 py-1 rounded-full text-xs font-semibold bg-zinc-100 dark:bg-zinc-800 text-zinc-600 dark:text-zinc-400">
              Age {userAgeInYear}
            </span>
          )}

          {isCurrentYear && (
            <span className="flex items-center gap-1 px-2.5 py-1 rounded-full text-xs font-semibold bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
              <Sparkles className="w-3 h-3 text-emerald-500" />
              <span>Current Year</span>
            </span>
          )}
        </div>

        {/* Jump to Today Button */}
        {!isCurrentYear && (
          <button
            onClick={() => onSelectYear(currentActualYear)}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-zinc-100 hover:bg-zinc-200 dark:bg-zinc-800 dark:hover:bg-zinc-700 text-zinc-700 dark:text-zinc-200 transition-colors self-start sm:self-auto"
          >
            <CalendarIcon className="w-3.5 h-3.5" />
            <span>Jump to {currentActualYear}</span>
          </button>
        )}
      </div>

      {/* 12 Months Calendar Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-5 pt-6">
        {monthsData.map(({ monthName, startingDayOfWeek, monthDays }) => (
          <div
            key={monthName}
            className="p-3 rounded-lg border border-zinc-100 dark:border-zinc-800/60 bg-zinc-50/50 dark:bg-zinc-900/40"
          >
            <h4 className="text-xs font-semibold text-zinc-800 dark:text-zinc-200 mb-2 px-1">
              {monthName}
            </h4>

            {/* Days of week header */}
            <div className="grid grid-cols-7 gap-1 text-center mb-1">
              {WEEKDAYS.map((wd) => (
                <span
                  key={wd}
                  className="text-[10px] font-medium text-zinc-400 dark:text-zinc-500 select-none"
                >
                  {wd}
                </span>
              ))}
            </div>

            {/* Day blocks grid */}
            <div className="grid grid-cols-7 gap-1">
              {/* Empty leading slots */}
              {Array.from({ length: startingDayOfWeek }).map((_, idx) => (
                <div key={`empty-${idx}`} className="aspect-square" />
              ))}

              {/* Month days */}
              {monthDays.map(({ dayNum, item }) => (
                <LifeDayBlock
                  key={item.date}
                  day={item}
                  dayNumber={dayNum}
                  isSelected={item.date === selectedDate}
                  onClick={onSelectDay}
                />
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};
