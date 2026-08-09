import React, { useEffect, useRef, useState } from 'react';
import { CalendarDays, ChevronLeft, ChevronRight } from 'lucide-react';
import { CustomSelect } from './CustomSelect';

/**
 * A calendar + time popover that replaces `input[type=datetime-local]`.
 *
 * The native control draws its own panel from the browser chrome, which no
 * stylesheet can reach — it stayed a blue-accented OS widget in the middle of
 * the app's neutral Geist surfaces. This renders the same value with the same
 * trigger and popover metrics as `CustomSelect`, so the field sits beside the
 * selects it shares a row with.
 *
 * `value` keeps the native `YYYY-MM-DDTHH:mm` shape, interpreted in the
 * viewer's own zone, so callers convert to and from UTC exactly as before.
 */
interface DateTimePickerProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  className?: string;
}

const WEEKDAY_INITIALS = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];

const HOUR_OPTIONS = Array.from({ length: 24 }, (_, h) => ({
  value: String(h),
  label: String(h).padStart(2, '0'),
}));

const MINUTE_OPTIONS = Array.from({ length: 60 }, (_, m) => ({
  value: String(m),
  label: String(m).padStart(2, '0'),
}));

/** The time a day gets when it is picked before any time is, so that choosing
 * a date alone never silently means midnight. */
const DEFAULT_HOUR = 9;

const pad = (n: number) => String(n).padStart(2, '0');

/** Formats a local `Date` as the `YYYY-MM-DDTHH:mm` this control exchanges. */
function toValue(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(
    d.getHours()
  )}:${pad(d.getMinutes())}`;
}

function parseValue(value: string): Date | null {
  if (!value) return null;
  const d = new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

function addMonths(d: Date, delta: number): Date {
  return new Date(d.getFullYear(), d.getMonth() + delta, 1);
}

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/**
 * The six-week grid whose first row contains the 1st of `month`. Fixed at 42
 * cells so the popover keeps one height across months.
 */
function monthGrid(month: Date): Date[] {
  const first = startOfMonth(month);
  const start = new Date(first.getFullYear(), first.getMonth(), 1 - first.getDay());
  return Array.from(
    { length: 42 },
    (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i)
  );
}

export const DateTimePicker: React.FC<DateTimePickerProps> = ({
  value,
  onChange,
  placeholder = 'Pick a date…',
  className = '',
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const [viewMonth, setViewMonth] = useState(() => startOfMonth(parseValue(value) ?? new Date()));
  const containerRef = useRef<HTMLDivElement>(null);

  const selected = parseValue(value);
  const today = new Date();

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setIsOpen(false);
    };

    document.addEventListener('mousedown', handleClickOutside);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  const open = () => {
    // The value may have changed since the last time this was open (a row
    // entering edit mode reseeds it), so start from the month it names.
    setViewMonth(startOfMonth(parseValue(value) ?? new Date()));
    setIsOpen(true);
  };

  const pickDay = (day: Date) => {
    const hour = selected ? selected.getHours() : DEFAULT_HOUR;
    const minute = selected ? selected.getMinutes() : 0;
    onChange(toValue(new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute)));
  };

  const pickTime = (hour: number, minute: number) => {
    const base = selected ?? today;
    onChange(toValue(new Date(base.getFullYear(), base.getMonth(), base.getDate(), hour, minute)));
  };

  const display = selected
    ? selected.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
    : placeholder;

  return (
    <div ref={containerRef} data-testid="datetime-picker" className={`relative ${className}`}>
      <button
        type="button"
        onClick={() => (isOpen ? setIsOpen(false) : open())}
        className="w-full flex items-center justify-between gap-2 px-3 py-2 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono text-neutral-900 dark:text-neutral-100 hover:bg-neutral-50 dark:hover:bg-neutral-850 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition shadow-xs cursor-pointer"
      >
        <span className={`truncate ${selected ? '' : 'text-neutral-400 dark:text-neutral-500'}`}>
          {display}
        </span>
        <CalendarDays className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
      </button>

      {isOpen && (
        <div
          data-testid="datetime-popover"
          className="absolute left-0 top-full mt-1.5 z-50 w-[272px] p-3 rounded-2xl bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-xl"
        >
          <div className="flex items-center justify-between pb-2">
            <span
              data-testid="datetime-month"
              data-month={`${viewMonth.getFullYear()}-${pad(viewMonth.getMonth() + 1)}`}
              className="text-xs font-medium text-neutral-900 dark:text-neutral-100"
            >
              {viewMonth.toLocaleString(undefined, { month: 'long', year: 'numeric' })}
            </span>
            <div className="flex items-center gap-1">
              <button
                type="button"
                aria-label="Previous month"
                onClick={() => setViewMonth(addMonths(viewMonth, -1))}
                className="p-1 rounded-lg text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-900 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              >
                <ChevronLeft className="w-3.5 h-3.5" />
              </button>
              <button
                type="button"
                aria-label="Next month"
                onClick={() => setViewMonth(addMonths(viewMonth, 1))}
                className="p-1 rounded-lg text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-900 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              >
                <ChevronRight className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          <div className="grid grid-cols-7 gap-1 pb-1">
            {WEEKDAY_INITIALS.map((initial, i) => (
              <span
                key={i}
                className="w-8 text-center text-[10px] font-mono text-neutral-400 dark:text-neutral-600"
              >
                {initial}
              </span>
            ))}
          </div>

          <div className="grid grid-cols-7 gap-1">
            {monthGrid(viewMonth).map((day) => {
              const isCurrentMonth = day.getMonth() === viewMonth.getMonth();
              const isSelected = selected != null && isSameDay(day, selected);
              const isToday = isSameDay(day, today);
              return (
                <button
                  key={day.getTime()}
                  type="button"
                  data-day={`${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}`}
                  onClick={() => pickDay(day)}
                  className={`w-8 h-8 rounded-lg text-xs font-mono transition cursor-pointer ${
                    isSelected
                      ? 'bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black font-medium'
                      : isCurrentMonth
                      ? 'text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-900'
                      : 'text-neutral-300 dark:text-neutral-700 hover:bg-neutral-100 dark:hover:bg-neutral-900'
                  } ${isToday && !isSelected ? 'ring-1 ring-neutral-300 dark:ring-neutral-700' : ''}`}
                >
                  {day.getDate()}
                </button>
              );
            })}
          </div>

          <div className="flex items-center gap-2 mt-3 pt-3 border-t border-neutral-200 dark:border-neutral-800">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Time</span>
            <div className="flex items-center gap-0.5 ml-auto">
              <div data-testid="datetime-hour">
                <CustomSelect
                  value={String(selected ? selected.getHours() : DEFAULT_HOUR)}
                  onChange={(h) => pickTime(Number(h), selected ? selected.getMinutes() : 0)}
                  options={HOUR_OPTIONS}
                  variant="inline"
                  mono
                />
              </div>
              <span className="text-xs font-mono text-neutral-400">:</span>
              <div data-testid="datetime-minute">
                <CustomSelect
                  value={String(selected ? selected.getMinutes() : 0)}
                  onChange={(m) => pickTime(selected ? selected.getHours() : DEFAULT_HOUR, Number(m))}
                  options={MINUTE_OPTIONS}
                  variant="inline"
                  mono
                />
              </div>
            </div>
          </div>

          <div className="flex items-center justify-between mt-3">
            <button
              type="button"
              onClick={() => {
                onChange('');
                setIsOpen(false);
              }}
              className="px-2 py-1 rounded-lg text-xs font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer"
            >
              Clear
            </button>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={() => {
                  const now = new Date();
                  setViewMonth(startOfMonth(now));
                  onChange(toValue(now));
                }}
                className="px-2 py-1 rounded-lg text-xs font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer"
              >
                Now
              </button>
              <button
                type="button"
                onClick={() => setIsOpen(false)}
                className="px-2.5 py-1 rounded-lg bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
