import React, { useState, useRef, useEffect, useId } from 'react';
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

function getAvailableSpace(el: HTMLElement) {
  const rect = el.getBoundingClientRect();
  let limitTop = 0;
  let limitBottom = window.innerHeight;

  let parent = el.parentElement;
  while (parent && parent !== document.body && parent !== document.documentElement) {
    const style = window.getComputedStyle(parent);
    const overflowY = style.overflowY;
    if (overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'hidden') {
      const parentRect = parent.getBoundingClientRect();
      if (parentRect.top > limitTop) limitTop = parentRect.top;
      if (parentRect.bottom < limitBottom) limitBottom = parentRect.bottom;
    }
    parent = parent.parentElement;
  }

  const spaceBelow = Math.max(0, limitBottom - rect.bottom);
  const spaceAbove = Math.max(0, rect.top - limitTop);
  return { spaceBelow, spaceAbove };
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
  const [highlightedIndex, setHighlightedIndex] = useState(-1);
  const [effectivePosition, setEffectivePosition] = useState<'bottom' | 'top'>('bottom');
  const [maxMenuHeight, setMaxMenuHeight] = useState<number | undefined>(undefined);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listboxId = useId();

  const selectedOption = options.find((opt) => opt.value === value);

  useEffect(() => {
    if (isOpen) {
      const idx = options.findIndex((opt) => opt.value === value);
      setHighlightedIndex(idx >= 0 ? idx : 0);
    } else {
      setHighlightedIndex(-1);
    }
  }, [isOpen, value, options]);

  useEffect(() => {
    if (isOpen && containerRef.current) {
      const { spaceBelow, spaceAbove } = getAvailableSpace(containerRef.current);
      if (position === 'top') {
        setEffectivePosition('top');
        setMaxMenuHeight(Math.min(240, Math.max(80, spaceAbove - 8)));
      } else if (position === 'bottom') {
        setEffectivePosition('bottom');
        setMaxMenuHeight(Math.min(240, Math.max(80, spaceBelow - 8)));
      } else {
        // Default to opening downward. Only flip upward if there is inadequate
        // space below (< 200px), MORE space above, AND at least 160px above.
        if (spaceBelow < 200 && spaceAbove > spaceBelow && spaceAbove >= 160) {
          setEffectivePosition('top');
          setMaxMenuHeight(Math.min(240, Math.max(80, spaceAbove - 8)));
        } else {
          setEffectivePosition('bottom');
          setMaxMenuHeight(Math.min(240, Math.max(80, spaceBelow - 8)));
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

  const handleTriggerKeyDown = (e: React.KeyboardEvent) => {
    if (!isOpen) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        setIsOpen(true);
      }
      return;
    }

    if (e.key === 'Escape') {
      e.preventDefault();
      setIsOpen(false);
      triggerRef.current?.focus();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlightedIndex((prev) => (prev + 1) % options.length);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlightedIndex((prev) => (prev - 1 + options.length) % options.length);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      if (highlightedIndex >= 0 && highlightedIndex < options.length) {
        onChange(options[highlightedIndex].value);
        setIsOpen(false);
        triggerRef.current?.focus();
      }
    }
  };

  const posClass = effectivePosition === 'top' ? 'bottom-full mb-1.5' : 'top-full mt-1.5';

  const triggerClasses =
    variant === 'inline'
      ? 'flex items-center gap-1 px-1.5 py-0.5 rounded text-xs text-neutral-900 dark:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-800 focus:outline-none transition cursor-pointer'
      : 'w-full h-9 flex items-center justify-between gap-2 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 text-xs text-neutral-900 dark:text-neutral-100 hover:bg-neutral-50 dark:hover:bg-neutral-850 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition shadow-xs cursor-pointer';

  const popoverClasses =
    variant === 'inline'
      ? `absolute left-0 ${posClass} w-auto min-w-[120px] z-50 p-1 rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-xl space-y-0.5 max-h-60 overflow-y-auto animate-in fade-in-0 zoom-in-95 duration-100`
      : `absolute left-0 ${posClass} w-full min-w-[180px] z-50 p-1 rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-xl space-y-0.5 max-h-60 overflow-y-auto animate-in fade-in-0 zoom-in-95 duration-100`;

  return (
    <div ref={containerRef} className={`relative ${variant === 'inline' ? 'inline-block' : 'w-full'} ${className}`}>
      {/* Trigger Button */}
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={isOpen}
        aria-haspopup="listbox"
        aria-controls={listboxId}
        aria-activedescendant={
          isOpen && highlightedIndex >= 0 ? `${listboxId}-option-${highlightedIndex}` : undefined
        }
        onClick={() => setIsOpen(!isOpen)}
        onKeyDown={handleTriggerKeyDown}
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
        <div
          id={listboxId}
          role="listbox"
          tabIndex={-1}
          className={popoverClasses}
          style={maxMenuHeight ? { maxHeight: `${maxMenuHeight}px` } : undefined}
        >
          {options.map((opt, idx) => {
            const isSelected = opt.value === value;
            const isHighlighted = idx === highlightedIndex;
            return (
              <div
                key={opt.value}
                id={`${listboxId}-option-${idx}`}
                role="option"
                aria-selected={isSelected}
                className="w-full"
              >
                <button
                  type="button"
                  onClick={() => {
                    onChange(opt.value);
                    setIsOpen(false);
                    triggerRef.current?.focus();
                  }}
                  onMouseEnter={() => setHighlightedIndex(idx)}
                  className={`w-full ${variant === 'inline' ? 'px-2.5 py-1.5 rounded text-xs' : 'px-3 py-2 rounded-md text-xs'} flex items-center justify-between gap-2 transition-colors cursor-pointer text-left ${
                    mono ? 'font-mono' : 'font-sans'
                  } ${
                    isSelected
                      ? 'bg-neutral-100 dark:bg-neutral-850 text-neutral-900 dark:text-neutral-100 font-medium'
                      : isHighlighted
                      ? 'bg-neutral-50 dark:bg-neutral-900 text-neutral-900 dark:text-neutral-100'
                      : 'text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-900 hover:text-neutral-900 dark:hover:text-neutral-100'
                  }`}
                >
                  <span className="truncate">{opt.label}</span>
                  {isSelected && showCheckmark && (
                    <Check className="w-3.5 h-3.5 text-neutral-900 dark:text-neutral-100 shrink-0" />
                  )}
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
