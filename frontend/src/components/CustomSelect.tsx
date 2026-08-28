import React, { useState, useRef, useEffect } from 'react';
import { ChevronDown, Check } from 'lucide-react';

export interface SelectOption {
  value: string;
  label: string;
}

interface CustomSelectProps {
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  placeholder?: string;
  className?: string;
  mono?: boolean;
  variant?: 'default' | 'inline';
  showCheckmark?: boolean;
  position?: 'bottom' | 'top' | 'auto';
}

export const CustomSelect: React.FC<CustomSelectProps> = ({
  value,
  onChange,
  options,
  placeholder = 'Select option...',
  className = '',
  mono = false,
  variant = 'default',
  showCheckmark = true,
  position = 'auto',
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const [effectivePosition, setEffectivePosition] = useState<'bottom' | 'top'>('bottom');
  const containerRef = useRef<HTMLDivElement>(null);

  const selectedOption = options.find((opt) => opt.value === value);

  useEffect(() => {
    if (isOpen && containerRef.current) {
      if (position === 'top') {
        setEffectivePosition('top');
      } else if (position === 'bottom') {
        setEffectivePosition('bottom');
      } else {
        const rect = containerRef.current.getBoundingClientRect();
        const dialog = containerRef.current.closest('[role="dialog"], form');
        const limitBottom = dialog ? dialog.getBoundingClientRect().bottom : window.innerHeight;
        const limitTop = dialog ? dialog.getBoundingClientRect().top : 0;
        const spaceBelow = limitBottom - rect.bottom;
        const spaceAbove = rect.top - limitTop;
        if (spaceBelow < 180 && spaceAbove > spaceBelow) {
          setEffectivePosition('top');
        } else {
          setEffectivePosition('bottom');
        }
      }
    }
  }, [isOpen, position]);

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

  const posClass = effectivePosition === 'top' ? 'bottom-full mb-1.5' : 'top-full mt-1.5';

  const triggerClasses =
    variant === 'inline'
      ? 'flex items-center gap-1 px-1.5 py-0.5 rounded text-xs text-neutral-900 dark:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-800 focus:outline-none transition cursor-pointer'
      : 'w-full flex items-center justify-between gap-2 px-3 py-2 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 text-xs text-neutral-900 dark:text-neutral-100 hover:bg-neutral-50 dark:hover:bg-neutral-850 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition shadow-xs cursor-pointer';

  const popoverClasses =
    variant === 'inline'
      ? `absolute left-0 ${posClass} w-auto min-w-[120px] z-50 p-1 rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-xl space-y-0.5 max-h-60 overflow-y-auto animate-in fade-in-0 zoom-in-95 duration-100`
      : `absolute left-0 ${posClass} w-full min-w-[180px] z-50 p-1 rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-xl space-y-0.5 max-h-60 overflow-y-auto animate-in fade-in-0 zoom-in-95 duration-100`;

  return (
    <div ref={containerRef} className={`relative inline-block ${className}`}>
      {/* Trigger Button */}
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className={`${triggerClasses} ${mono ? 'font-mono' : 'font-sans'}`}
      >
        <span className="truncate">
          {selectedOption ? selectedOption.label : placeholder}
        </span>
        <ChevronDown
          className={`w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500 transition-transform duration-150 shrink-0 ${
            isOpen ? 'rotate-180' : ''
          }`}
        />
      </button>

      {/* Floating Popover Dropdown Menu */}
      {isOpen && (
        <div className={popoverClasses}>
          {options.map((opt) => {
            const isSelected = opt.value === value;
            return (
              <button
                key={opt.value}
                type="button"
                onClick={() => {
                  onChange(opt.value);
                  setIsOpen(false);
                }}
                className={`w-full ${variant === 'inline' ? 'px-2.5 py-1.5 rounded text-xs' : 'px-3 py-2 rounded-md text-xs'} flex items-center justify-between gap-2 transition-colors cursor-pointer text-left ${
                  mono ? 'font-mono' : 'font-sans'
                } ${
                  isSelected
                    ? 'bg-neutral-100 dark:bg-neutral-850 text-neutral-900 dark:text-neutral-100 font-medium'
                    : 'text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-900 hover:text-neutral-900 dark:hover:text-neutral-100'
                }`}
              >
                <span className="truncate">{opt.label}</span>
                {isSelected && showCheckmark && (
                  <Check className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400 shrink-0" />
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
};
