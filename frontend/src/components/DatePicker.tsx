import React, { useEffect, useRef, useState, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { CalendarDays, ChevronLeft, ChevronRight, X } from 'lucide-react';

export interface DatePickerProps {
  value: string; // 'YYYY-MM-DD' or ''
  onChange: (value: string) => void;
  placeholder?: string;
  className?: string;
  min?: string; // 'YYYY-MM-DD'
  max?: string; // 'YYYY-MM-DD'
  disabled?: boolean;
  required?: boolean;
  id?: string;
  size?: 'sm' | 'default';
  align?: 'left' | 'right';
  displayValue?: string;
}

const WEEKDAY_INITIALS = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];

const pad = (n: number) => String(n).padStart(2, '0');

/** Formats a local Date as YYYY-MM-DD string */
function formatIsoDate(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Parses YYYY-MM-DD string into a safe Date at noon to avoid timezone shifts */
function parseIsoDate(val: string): Date | null {
  if (!val) return null;
  const parts = val.split('-').map(Number);
  if (parts.length < 3 || isNaN(parts[0]) || isNaN(parts[1]) || isNaN(parts[2])) {
    return null;
  }
  return new Date(parts[0], parts[1] - 1, parts[2], 12, 0, 0);
}

function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1, 12, 0, 0);
}

function addMonths(d: Date, delta: number): Date {
  return new Date(d.getFullYear(), d.getMonth() + delta, 1, 12, 0, 0);
}

/**
 * 42-day calendar grid (6 weeks) starting on Sunday
 */
function monthGrid(month: Date): Date[] {
  const first = startOfMonth(month);
  const start = new Date(first.getFullYear(), first.getMonth(), 1 - first.getDay(), 12, 0, 0);
  return Array.from(
    { length: 42 },
    (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i, 12, 0, 0)
  );
}

export const DatePicker: React.FC<DatePickerProps> = ({
  value,
  onChange,
  placeholder = 'Select date…',
  className = '',
  min,
  max,
  disabled = false,
  required = false,
  id,
  size = 'default',
  align = 'left',
  displayValue,
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const [isYearPickerOpen, setIsYearPickerOpen] = useState(false);
  const [coords, setCoords] = useState<{ top: number; left: number } | null>(null);

  const parsedValue = useMemo(() => parseIsoDate(value), [value]);
  const [viewMonth, setViewMonth] = useState(() =>
    startOfMonth(parsedValue ?? new Date())
  );

  const containerRef = useRef<HTMLDivElement>(null);
  const hiddenInputRef = useRef<HTMLInputElement>(null);

  const today = useMemo(() => {
    const now = new Date();
    return new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12, 0, 0);
  }, []);
  const todayStr = useMemo(() => formatIsoDate(today), [today]);

  useEffect(() => {
    if (parsedValue) {
      setViewMonth(startOfMonth(parsedValue));
    }
  }, [parsedValue]);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        const target = e.target as HTMLElement;
        if (target.closest('[data-testid="datepicker-popover"]')) return;
        setIsOpen(false);
        setIsYearPickerOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setIsOpen(false);
        setIsYearPickerOpen(false);
      }
    };

    document.addEventListener('mousedown', handleClickOutside);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  useEffect(() => {
    if (!isOpen) return;

    const updatePosition = () => {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const popoverHeight = 290;
      const popoverWidth = 260;
      const margin = 8;
      const vh = window.innerHeight;
      const vw = window.innerWidth;

      let top = rect.bottom + 6;
      if (top + popoverHeight > vh - margin) {
        const topUp = rect.top - popoverHeight - 6;
        top = topUp >= margin ? topUp : Math.max(margin, vh - popoverHeight - margin);
      }

      let left = align === 'right' ? rect.right - popoverWidth : rect.left;
      if (left < margin) {
        left = margin;
      } else if (left + popoverWidth > vw - margin) {
        left = Math.max(margin, vw - popoverWidth - margin);
      }

      setCoords({ top, left });
    };

    updatePosition();
    window.addEventListener('resize', updatePosition);
    window.addEventListener('scroll', updatePosition, true);
    return () => {
      window.removeEventListener('resize', updatePosition);
      window.removeEventListener('scroll', updatePosition, true);
    };
  }, [isOpen]);

  const handlePickDay = (day: Date) => {
    const iso = formatIsoDate(day);
    if (min && iso < min) return;
    if (max && iso > max) return;
    onChange(iso);
    setIsOpen(false);
    setIsYearPickerOpen(false);
  };

  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation();
    onChange('');
    setIsOpen(false);
  };

  const handleSetYear = (year: number) => {
    setViewMonth(new Date(year, viewMonth.getMonth(), 1, 12, 0, 0));
    setIsYearPickerOpen(false);
  };

  const isDayDisabled = (d: Date) => {
    const iso = formatIsoDate(d);
    if (min && iso < min) return true;
    if (max && iso > max) return true;
    return false;
  };

  const currentYear = viewMonth.getFullYear();
  const yearOptions = useMemo(() => {
    const years: number[] = [];
    const base = new Date().getFullYear();
    const startYr = min ? parseInt(min.slice(0, 4), 10) : base - 100;
    const endYr = max ? parseInt(max.slice(0, 4), 10) : base + 20;
    for (let y = endYr; y >= startYr; y--) {
      years.push(y);
    }
    return years;
  }, [min, max]);

  return (
    <div ref={containerRef} data-testid="datepicker-container" className={`relative ${className.includes('w-') ? '' : 'w-full'} ${className}`}>
      {/* Hidden input for form validation, accessible label binding & automated test fill compatibility */}
      <input
        ref={hiddenInputRef}
        type="date"
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        min={min}
        max={max}
        required={required}
        disabled={disabled}
        className="sr-only"
        tabIndex={-1}
        aria-hidden="true"
      />

      {/* Visible trigger button */}
      <button
        type="button"
        disabled={disabled}
        onClick={() => {
          if (disabled) return;
          setIsOpen((prev) => !prev);
          setIsYearPickerOpen(false);
        }}
        className={`w-full ${size === 'sm' ? 'h-8 px-2.5' : 'h-9 px-3'} flex items-center justify-between gap-2 rounded-md text-xs border border-neutral-200 dark:border-neutral-700 bg-neutral-50 dark:bg-neutral-800/60 text-neutral-900 dark:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-700/60 focus:outline-hidden focus:ring-1 focus:ring-neutral-900 dark:focus:ring-neutral-100 transition shadow-xs cursor-pointer ${
          disabled ? 'opacity-50 cursor-not-allowed' : ''
        }`}
      >
        <span className={`truncate font-mono ${value ? 'font-medium' : 'text-neutral-400 dark:text-neutral-500 font-normal'}`}>
          {displayValue || value || placeholder}
        </span>
        <div className="flex items-center gap-1 shrink-0">
          {value && !required && !disabled && (
            <span
              role="button"
              tabIndex={0}
              aria-label="Clear date"
              onClick={handleClear}
              className="p-0.5 rounded hover:bg-neutral-200 dark:hover:bg-neutral-700 text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200"
            >
              <X className="w-3 h-3" />
            </span>
          )}
          <CalendarDays className="w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500" />
        </div>
      </button>

      {/* Floating Popover Portal */}
      {isOpen && coords && createPortal(
        <div
          data-testid="datepicker-popover"
          style={{
            position: 'fixed',
            top: `${coords.top}px`,
            left: `${coords.left}px`,
            zIndex: 99999,
          }}
          className="w-[260px] p-3 rounded-xl bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-xl font-sans animate-in fade-in zoom-in-95 duration-100 select-none"
        >
          {/* Header */}
          <div className="flex items-center justify-between pb-2 mb-1 border-b border-neutral-100 dark:border-neutral-800">
            <div className="flex items-center gap-1">
              <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                {viewMonth.toLocaleString(undefined, { month: 'short' })}
              </span>
              <button
                type="button"
                onClick={() => setIsYearPickerOpen((prev) => !prev)}
                className="px-1.5 py-0.5 rounded text-xs font-mono font-semibold text-neutral-800 dark:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
              >
                {currentYear} ▾
              </button>
            </div>

            <div className="flex items-center gap-0.5">
              <button
                type="button"
                aria-label="Previous month"
                onClick={() => {
                  setViewMonth(addMonths(viewMonth, -1));
                  setIsYearPickerOpen(false);
                }}
                className="p-1 rounded text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              >
                <ChevronLeft className="w-3.5 h-3.5" />
              </button>
              <button
                type="button"
                aria-label="Next month"
                onClick={() => {
                  setViewMonth(addMonths(viewMonth, 1));
                  setIsYearPickerOpen(false);
                }}
                className="p-1 rounded text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              >
                <ChevronRight className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          {/* Year selector scroll list if opened */}
          {isYearPickerOpen ? (
            <div className="h-[180px] overflow-y-auto pr-1 space-y-0.5">
              {yearOptions.map((yr) => (
                <button
                  key={yr}
                  type="button"
                  onClick={() => handleSetYear(yr)}
                  className={`w-full text-left px-2.5 py-1 text-xs font-mono rounded transition cursor-pointer ${
                    yr === currentYear
                      ? 'bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-semibold'
                      : 'hover:bg-neutral-100 dark:hover:bg-neutral-800 text-neutral-700 dark:text-neutral-300'
                  }`}
                >
                  {yr}
                </button>
              ))}
            </div>
          ) : (
            <>
              {/* Weekday initials */}
              <div className="grid grid-cols-7 gap-1 text-center pb-1">
                {WEEKDAY_INITIALS.map((init, idx) => (
                  <span key={idx} className="text-[10px] font-medium text-neutral-400 dark:text-neutral-500">
                    {init}
                  </span>
                ))}
              </div>

              {/* 42-day Month Grid */}
              <div className="grid grid-cols-7 gap-1">
                {monthGrid(viewMonth).map((d) => {
                  const iso = formatIsoDate(d);
                  const isCurrentMonth = d.getMonth() === viewMonth.getMonth();
                  const isSelected = iso === value;
                  const isToday = iso === todayStr;
                  const disabledDay = isDayDisabled(d);

                  let btnCls = 'text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800';
                  if (!isCurrentMonth) {
                    btnCls = 'text-neutral-300 dark:text-neutral-600 hover:bg-neutral-50 dark:hover:bg-neutral-800/60';
                  }
                  if (isSelected) {
                    btnCls = 'bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-semibold shadow-xs';
                  } else if (isToday) {
                    btnCls += ' ring-1 ring-emerald-500 font-medium';
                  }
                  if (disabledDay) {
                    btnCls = 'text-neutral-200 dark:text-neutral-700 opacity-40 cursor-not-allowed';
                  }

                  return (
                    <button
                      key={iso}
                      type="button"
                      disabled={disabledDay}
                      onClick={() => handlePickDay(d)}
                      className={`h-7 w-full flex items-center justify-center rounded text-xs transition cursor-pointer ${btnCls}`}
                    >
                      {d.getDate()}
                    </button>
                  );
                })}
              </div>

              {/* Footer actions: Quick Today jump */}
              <div className="flex items-center justify-between pt-2 mt-2 border-t border-neutral-100 dark:border-neutral-800 text-[11px]">
                <button
                  type="button"
                  onClick={() => {
                    if (isDayDisabled(today)) return;
                    handlePickDay(today);
                  }}
                  className="text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 font-medium transition cursor-pointer"
                >
                  Today
                </button>
                {value && (
                  <button
                    type="button"
                    onClick={() => {
                      onChange('');
                      setIsOpen(false);
                    }}
                    className="text-neutral-400 hover:text-rose-500 transition cursor-pointer"
                  >
                    Clear
                  </button>
                )}
              </div>
            </>
          )}
        </div>,
        document.body
      )}
    </div>
  );
};
