import React, { useState, useEffect, useRef } from 'react';
import { fetchApi } from '../../core/apiClient';
import { TickerSearchResult } from './types';
import { Search, Loader2 } from 'lucide-react';

interface TickerSearchInputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  required?: boolean;
  className?: string;
}

export const TickerSearchInput: React.FC<TickerSearchInputProps> = ({
  value,
  onChange,
  placeholder = 'e.g. AAPL, BTC, VOO',
  required = false,
  className = '',
}) => {
  const [query, setQuery] = useState(value);
  const [results, setResults] = useState<TickerSearchResult[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const containerRef = useRef<HTMLDivElement>(null);

  // Sync external value changes
  useEffect(() => {
    setQuery(value);
  }, [value]);

  // Debounced search
  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < 1) {
      setResults([]);
      setIsLoading(false);
      return;
    }

    setIsLoading(true);
    const timer = setTimeout(async () => {
      try {
        const data = await fetchApi<TickerSearchResult[]>(
          `/portfolio/search?q=${encodeURIComponent(trimmed)}`
        );
        setResults(data || []);
      } catch (err) {
        console.error('Ticker search failed:', err);
        setResults([]);
      } finally {
        setIsLoading(false);
      }
    }, 250);

    return () => clearTimeout(timer);
  }, [query]);

  // Handle outside click to close dropdown
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const handleSelect = (symbol: string) => {
    onChange(symbol);
    setQuery(symbol);
    setIsOpen(false);
    setSelectedIndex(-1);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (!isOpen || results.length === 0) return;

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((prev) => (prev < results.length - 1 ? prev + 1 : 0));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((prev) => (prev > 0 ? prev - 1 : results.length - 1));
    } else if (e.key === 'Enter') {
      if (selectedIndex >= 0 && selectedIndex < results.length) {
        e.preventDefault();
        handleSelect(results[selectedIndex].symbol);
      }
    } else if (e.key === 'Escape') {
      setIsOpen(false);
    }
  };

  return (
    <div ref={containerRef} className="relative w-full">
      <div className="relative">
        <input
          type="text"
          value={query}
          onChange={(e) => {
            const next = e.target.value;
            setQuery(next);
            onChange(next);
            setIsOpen(true);
            setSelectedIndex(-1);
          }}
          onFocus={() => setIsOpen(true)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          required={required}
          className={`w-full pr-8 ${className}`}
        />
        <div className="absolute right-2.5 top-1/2 -translate-y-1/2 text-neutral-400 pointer-events-none">
          {isLoading ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Search className="w-3.5 h-3.5 opacity-60" />
          )}
        </div>
      </div>

      {/* Autocomplete Dropdown */}
      {isOpen && results.length > 0 && (
        <div className="absolute z-50 mt-1 w-full max-h-56 overflow-y-auto rounded-md bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-lg font-sans text-xs divide-y divide-neutral-100 dark:divide-neutral-800/60">
          {results.map((item, idx) => (
            <div
              key={`${item.symbol}-${idx}`}
              onMouseDown={() => handleSelect(item.symbol)}
              onMouseEnter={() => setSelectedIndex(idx)}
              className={`px-3 py-2 cursor-pointer transition flex items-center justify-between gap-2 ${
                selectedIndex === idx
                  ? 'bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100'
                  : 'hover:bg-neutral-50 dark:hover:bg-neutral-800/50 text-neutral-700 dark:text-neutral-300'
              }`}
            >
              <div className="flex flex-col min-w-0 pr-2">
                <div className="flex items-center gap-1.5">
                  <span className="font-mono font-bold text-neutral-900 dark:text-neutral-100">
                    {item.symbol}
                  </span>
                  {item.exchange && (
                    <span className="px-1.5 py-0.2 rounded text-[10px] font-mono bg-neutral-200 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
                      {item.exchange}
                    </span>
                  )}
                </div>
                <span className="text-[11px] text-neutral-500 truncate max-w-[240px]">
                  {item.name}
                </span>
              </div>

              {item.quote_type && (
                <span className="text-[10px] font-mono text-neutral-400 uppercase shrink-0">
                  {item.quote_type}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
