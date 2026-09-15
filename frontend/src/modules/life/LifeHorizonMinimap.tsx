import React, { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import type { LifeHorizonDay } from './types';
import { MOOD_CONFIG } from './types';

interface LifeHorizonMinimapProps {
  days: LifeHorizonDay[];
  onSelectDate: (date: string) => void;
  isDark?: boolean;
}

export const LifeHorizonMinimap: React.FC<LifeHorizonMinimapProps> = ({
  days,
  onSelectDate,
  isDark: isDarkProp,
}) => {
  const [isDark, setIsDark] = useState(() => {
    if (typeof isDarkProp === 'boolean') return isDarkProp;
    return typeof document !== 'undefined' && document.documentElement.classList.contains('dark');
  });

  useEffect(() => {
    const checkTheme = () => {
      if (typeof isDarkProp === 'boolean') {
        setIsDark(isDarkProp);
      } else {
        setIsDark(document.documentElement.classList.contains('dark'));
      }
    };
    checkTheme();

    const root = document.documentElement;
    const observer = new MutationObserver(checkTheme);
    observer.observe(root, { attributes: true, attributeFilter: ['class'] });
    return () => observer.disconnect();
  }, [isDarkProp]);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const baseCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const tooltipDateRef = useRef<HTMLSpanElement | null>(null);
  const tooltipBadgeRef = useRef<HTMLSpanElement | null>(null);
  const tooltipTimeRef = useRef<HTMLSpanElement | null>(null);

  const animFrameRef = useRef<number | null>(null);

  // Simple hover tracking
  const hoveredCellRef = useRef<{ col: number | null; row: number | null }>({ col: null, row: null });

  // Extract distinct decades for quick jump bar
  const decades = useMemo(() => {
    const set = new Set<number>();
    for (const d of days) {
      set.add(Math.floor(d.year / 10) * 10);
    }
    return Array.from(set).sort((a, b) => a - b);
  }, [days]);

  // Determine current decade matching today's present day or current calendar year
  const currentDecade = useMemo(() => {
    const presentDay = days.find((d) => d.status === 'present');
    if (presentDay) {
      return Math.floor(presentDay.year / 10) * 10;
    }
    const currentYearDecade = Math.floor(new Date().getFullYear() / 10) * 10;
    if (decades.includes(currentYearDecade)) {
      return currentYearDecade;
    }
    return decades.length > 0 ? decades[0] : null;
  }, [days, decades]);

  const [selectedDecade, setSelectedDecade] = useState<number | null>(() => {
    const presentDay = days.find((d) => d.status === 'present');
    if (presentDay) {
      return Math.floor(presentDay.year / 10) * 10;
    }
    const currentYearDecade = Math.floor(new Date().getFullYear() / 10) * 10;
    for (const d of days) {
      if (Math.floor(d.year / 10) * 10 === currentYearDecade) {
        return currentYearDecade;
      }
    }
    return null;
  });

  const hasInitializedRef = useRef(false);
  useEffect(() => {
    if (!hasInitializedRef.current && currentDecade !== null) {
      setSelectedDecade(currentDecade);
      hasInitializedRef.current = true;
    }
  }, [currentDecade]);

  const currentColsRef = useRef(100);
  const currentRowsRef = useRef(100);
  const currentCellWidthRef = useRef(4);
  const currentCellHeightRef = useRef(4);
  const currentWRef = useRef(0);
  const currentHRef = useRef(0);

  // Filter days by selected decade if active
  const displayedDays = useMemo(() => {
    if (selectedDecade === null) return days;
    return days.filter((d) => Math.floor(d.year / 10) * 10 === selectedDecade);
  }, [days, selectedDecade]);

  // 1. Pre-render base grid into offscreen canvas ONLY on size / filter changes
  const renderBaseGrid = useCallback(
    (cols: number, rows: number, cellWidth: number, cellHeight: number, w: number, h: number) => {
      if (displayedDays.length === 0 || w <= 0 || h <= 0) return;

      if (!baseCanvasRef.current) {
        baseCanvasRef.current = document.createElement('canvas');
      }
      const baseCanvas = baseCanvasRef.current;
      const dpr = window.devicePixelRatio || 1;

      baseCanvas.width = Math.floor(w * dpr);
      baseCanvas.height = Math.floor(h * dpr);

      const ctx = baseCanvas.getContext('2d', { alpha: false });
      if (!ctx) return;

      ctx.save();
      ctx.scale(dpr, dpr);

      // Background fill
      ctx.fillStyle = isDark ? '#09090b' : '#ffffff';
      ctx.fillRect(0, 0, w, h);

      // Theme colors following Design.md
      const emptyPastColor = isDark ? '#27272a' : '#e4e4e7';
      const futureColor = isDark ? '#141417' : '#fafafa';
      const futureBorderColor = isDark ? '#2e2e33' : '#e4e4e7';
      const presentColor = '#3b82f6';

      const total = displayedDays.length;
      const pad = Math.max(0.6, Math.min(1.2, cellWidth * 0.16));
      const bw = Math.max(1, cellWidth - pad);
      const bh = Math.max(1, cellHeight - pad);

      ctx.globalAlpha = 1.0;

      for (let i = 0; i < total; i++) {
        const d = displayedDays[i];
        const col = i % cols;
        const row = Math.floor(i / cols);

        const x = col * cellWidth;
        const y = row * cellHeight;

        if (d.status === 'present') {
          ctx.fillStyle = presentColor;
          ctx.fillRect(x, y, bw, bh);
          ctx.strokeStyle = '#60a5fa';
          ctx.lineWidth = 1;
          ctx.strokeRect(x, y, bw, bh);
        } else if (d.status === 'past_logged' && d.mood) {
          const moodKey = d.mood.toLowerCase() as keyof typeof MOOD_CONFIG;
          ctx.fillStyle = MOOD_CONFIG[moodKey]?.color || '#10b981';
          ctx.fillRect(x, y, bw, bh);
          if (d.has_activity && bw >= 6) {
            ctx.fillStyle = '#6366f1';
            ctx.fillRect(x + bw - 2.5, y + 1, 1.5, 1.5);
          }
        } else if (d.status === 'past_activity' || (!d.mood && d.has_activity)) {
          ctx.fillStyle = '#6366f1';
          ctx.fillRect(x, y, bw, bh);
        } else if (d.status === 'past_empty') {
          ctx.fillStyle = emptyPastColor;
          ctx.fillRect(x, y, bw, bh);
        } else {
          ctx.fillStyle = futureColor;
          ctx.fillRect(x, y, bw, bh);
          ctx.strokeStyle = futureBorderColor;
          ctx.lineWidth = 0.6;
          ctx.strokeRect(x, y, bw, bh);
        }
      }

      ctx.restore();
    },
    [displayedDays, isDark]
  );

  // 2. Render Scene: Blit cached base grid & apply macOS Dock-style 2D radial hover magnification
  const renderScene = useCallback(
    (hoveredCol: number | null, hoveredRow: number | null) => {
      const canvas = canvasRef.current;
      const baseCanvas = baseCanvasRef.current;
      if (!canvas || !baseCanvas) return;

      const ctx = canvas.getContext('2d');
      if (!ctx) return;

      const dpr = window.devicePixelRatio || 1;
      const w = currentWRef.current;
      const h = currentHRef.current;
      if (w <= 0 || h <= 0) return;

      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.clearRect(0, 0, w, h);

      // Blit full pre-rendered base grid
      ctx.drawImage(baseCanvas, 0, 0, w, h);

      // Draw hover highlight ring around the hovered cell
      if (hoveredCol !== null && hoveredRow !== null) {
        const cols = currentColsRef.current;
        const idx = hoveredRow * cols + hoveredCol;
        if (idx >= 0 && idx < displayedDays.length) {
          const cellWidth = currentCellWidthRef.current;
          const cellHeight = currentCellHeightRef.current;
          const pad = 1;
          const bx = hoveredCol * cellWidth + pad;
          const by = hoveredRow * cellHeight + pad;
          const bw = cellWidth - pad * 2;
          const bh = cellHeight - pad * 2;

          ctx.globalAlpha = 1.0;
          ctx.strokeStyle = isDark ? '#ffffff' : '#09090b';
          ctx.lineWidth = 1.5;
          ctx.strokeRect(bx, by, bw, bh);
        }
      }

      ctx.restore();
    },
    [displayedDays.length, isDark]
  );

  // 3. Update canvas dimensions — 2x block size, square cells, no overflow scroll
  const updateSize = useCallback(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas || displayedDays.length === 0) return;

    const containerRect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.floor(containerRect.width);
    if (w <= 0) return;

    const total = displayedDays.length;

    // For full lifespan (~29,220 days), target ~9.5px blocks (2X size).
    // For a single decade (~3,652 days), target ~14px blocks so they fill the view nicely.
    const isFiltered = selectedDecade !== null;
    const targetCellSize = isFiltered ? 14 : 9.5;
    const cols = Math.max(1, Math.floor(w / targetCellSize));
    const rows = Math.ceil(total / cols);
    const cellWidth = w / cols;          // exact to fill width
    const cellHeight = cellWidth;        // square cells
    const h = Math.ceil(rows * cellHeight);

    currentWRef.current = w;
    currentHRef.current = h;
    currentColsRef.current = cols;
    currentRowsRef.current = rows;
    currentCellWidthRef.current = cellWidth;
    currentCellHeightRef.current = cellHeight;

    canvas.width = Math.floor(w * dpr);
    canvas.height = Math.floor(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;

    renderBaseGrid(cols, rows, cellWidth, cellHeight, w, h);
    renderScene(hoveredCellRef.current.col, hoveredCellRef.current.row);
  }, [displayedDays.length, selectedDecade, renderBaseGrid, renderScene]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    updateSize();
    const ro = new ResizeObserver(updateSize);
    ro.observe(container);

    return () => ro.disconnect();
  }, [updateSize]);

  // Update tooltip content directly in DOM (zero React re-render)
  const updateTooltipDOM = useCallback((d: LifeHorizonDay) => {
    if (tooltipDateRef.current) tooltipDateRef.current.textContent = d.date;
    if (tooltipBadgeRef.current) {
      const el = tooltipBadgeRef.current;
      if (d.status === 'present') {
        el.className = 'px-1.5 py-0.5 rounded text-[10px] font-bold bg-blue-500 text-white';
        el.style.backgroundColor = '';
        el.textContent = 'TODAY';
      } else if (d.mood) {
        const moodKey = d.mood.toLowerCase() as keyof typeof MOOD_CONFIG;
        const conf = MOOD_CONFIG[moodKey];
        el.className = 'px-1.5 py-0.5 rounded text-[10px] font-bold text-white shadow-xs capitalize';
        el.style.backgroundColor = conf?.color || '#10b981';
        el.textContent = `${conf?.emoji ?? '✨'} ${d.mood}`;
      } else if (d.status === 'past_activity' || d.has_activity) {
        el.className = 'px-1.5 py-0.5 rounded text-[10px] font-bold text-white shadow-xs';
        el.style.backgroundColor = '#6366f1';
        const cnt = d.activity_count || 1;
        el.textContent = `⚡ ${cnt} ${cnt === 1 ? 'Activity' : 'Activities'}`;
      } else if (d.status === 'past_empty') {
        el.className = 'px-1.5 py-0.5 rounded text-[10px] bg-neutral-700 dark:bg-neutral-300 text-neutral-200 dark:text-neutral-800';
        el.style.backgroundColor = '';
        el.textContent = 'Unrecorded';
      } else {
        el.className = 'px-1.5 py-0.5 rounded text-[10px] bg-neutral-800 dark:bg-neutral-200 text-neutral-300 dark:text-neutral-700';
        el.style.backgroundColor = '';
        el.textContent = 'Future';
      }
    }

    if (tooltipTimeRef.current) {
      let timeSummary = '';
      if (d.time_allocation) {
        try {
          const blocks: (string | null)[] = JSON.parse(d.time_allocation);
          let deepWork = 0;
          let sleep = 0;
          for (const b of blocks) {
            if (b === 'deep_work') deepWork += 1;
            else if (b === 'sleep') sleep += 1;
          }
          const parts = [];
          if (deepWork > 0) parts.push(`⚡ ${(deepWork * 0.25).toFixed(1)}h`);
          if (sleep > 0) parts.push(`💤 ${(sleep * 0.25).toFixed(1)}h`);
          timeSummary = parts.join(' · ');
        } catch {
          timeSummary = '';
        }
      }
      if (d.has_activity && d.mood) {
        const cnt = d.activity_count || 1;
        const actLabel = `📝 ${cnt} ${cnt === 1 ? 'act' : 'acts'}`;
        timeSummary = timeSummary ? `${timeSummary} · ${actLabel}` : actLabel;
      }
      tooltipTimeRef.current.textContent = timeSummary;
      tooltipTimeRef.current.style.display = timeSummary ? 'inline' : 'none';
    }
  }, []);

  // 4. Ultra-Fast Pointer Move (ZERO React re-renders)
  const handlePointerMove = (clientX: number, clientY: number) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Always get a live rect — getBoundingClientRect() is viewport-relative and
    // accounts for current scroll position, so it stays correct as the page scrolls.
    const rect = canvas.getBoundingClientRect();

    const mouseX = clientX - rect.left;
    const mouseY = clientY - rect.top;

    const cols = currentColsRef.current;
    const rows = currentRowsRef.current;
    const cellWidth = currentCellWidthRef.current;
    const cellHeight = currentCellHeightRef.current;

    const col = Math.floor(mouseX / cellWidth);
    const row = Math.floor(mouseY / cellHeight);

    if (col < 0 || col >= cols || row < 0 || row >= rows) {
      handlePointerLeave();
      return;
    }

    const idx = row * cols + col;
    if (idx >= 0 && idx < displayedDays.length) {
      hoveredCellRef.current = { col, row };

      // Update tooltip position directly via hardware transform (clamped to never clip edges)
      if (tooltipRef.current) {
        const tooltipHalfWidth = 145;
        const left = Math.max(
          tooltipHalfWidth + 12,
          Math.min(currentWRef.current - tooltipHalfWidth - 12, mouseX)
        );
        const isNearTop = mouseY < 75;
        const top = isNearTop ? mouseY + 24 : mouseY - 14;

        tooltipRef.current.style.display = 'block';
        tooltipRef.current.style.transform = `translate3d(${left}px, ${top}px, 0) translateY(${isNearTop ? '0%' : '-100%'}) translateX(-50%)`;
      }

      updateTooltipDOM(displayedDays[idx]);

      // Schedule single RAF frame for rendering
      if (!animFrameRef.current) {
        animFrameRef.current = requestAnimationFrame(() => {
          animFrameRef.current = null;
          const hovered = hoveredCellRef.current;
          renderScene(hovered.col, hovered.row);
        });
      }
    } else {
      handlePointerLeave();
    }
  };

  const handlePointerEnter = () => { /* live rect fetched in handlePointerMove */ };

  const handlePointerLeave = () => {
    hoveredCellRef.current = { col: null, row: null };

    if (tooltipRef.current) {
      tooltipRef.current.style.display = 'none';
    }

    if (animFrameRef.current) {
      cancelAnimationFrame(animFrameRef.current);
      animFrameRef.current = null;
    }
    renderScene(null, null);
  };

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas || displayedDays.length === 0) return;
    const rect = canvas.getBoundingClientRect();
    const mouseX = e.clientX - rect.left;
    const mouseY = e.clientY - rect.top;

    const cols = currentColsRef.current;
    const cellWidth = currentCellWidthRef.current;
    const cellHeight = currentCellHeightRef.current;

    const col = Math.floor(mouseX / cellWidth);
    const row = Math.floor(mouseY / cellHeight);
    const idx = row * cols + col;

    if (idx >= 0 && idx < displayedDays.length) {
      onSelectDate(displayedDays[idx].date);
    }
  };

  const handleSelectDecade = (decade: number | null) => {
    setSelectedDecade((prev) => (prev === decade ? null : decade));
    hoveredCellRef.current = { col: null, row: null };
    if (tooltipRef.current) {
      tooltipRef.current.style.display = 'none';
    }
  };

  const handleSelectToday = () => {
    const todayStr = new Date().toISOString().split('T')[0];
    const currentYearDecade = Math.floor(new Date().getFullYear() / 10) * 10;
    if (selectedDecade !== currentYearDecade) {
      setSelectedDecade(currentYearDecade);
    }
    onSelectDate(todayStr);
  };

  if (days.length === 0) return null;

  return (
    <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 space-y-4">
      {/* Header & Controls */}
      <div className="flex flex-col md:flex-row md:items-baseline justify-between gap-3 pb-3 border-b border-neutral-100 dark:border-neutral-800">
        <div>
          <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
            Lifespan day grid {selectedDecade ? `— ${selectedDecade}s` : ''}
          </h2>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-0.5">
            {displayedDays.length.toLocaleString()} total days{selectedDecade ? ` in the ${selectedDecade}s` : ''}. Hover to inspect and click any day to view or record.
          </p>
        </div>

        {/* Legend */}
        <div className="flex flex-wrap items-center gap-3 text-xs text-neutral-500 dark:text-neutral-400">
          {[
            { label: 'Past', cls: 'bg-neutral-200 dark:bg-neutral-800' },
            { label: 'Today', cls: 'bg-blue-500 ring-1 ring-blue-400' },
            { label: 'Auto-Tracked', cls: 'bg-indigo-500' },
            { label: 'Amazing', cls: 'bg-amber-400 dark:bg-amber-500' },
            { label: 'Good', cls: 'bg-emerald-500' },
            { label: 'Neutral', cls: 'bg-cyan-500' },
            { label: 'Low', cls: 'bg-purple-500' },
            { label: 'Rough', cls: 'bg-rose-500' },
            { label: 'Future', cls: 'bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800' },
          ].map((item) => (
            <div key={item.label} className="flex items-center gap-1.5">
              <span className={`w-2.5 h-2.5 rounded-xs ${item.cls}`} />
              <span>{item.label}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Decade Quick-Jump Bar & Jump to Today */}
      <div className="flex items-center justify-between gap-2 overflow-x-auto pb-1 text-xs">
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-xs text-neutral-400 dark:text-neutral-500 mr-1">
            Filter decade:
          </span>
          <button
            type="button"
            onClick={() => handleSelectDecade(null)}
            className={`px-2 py-0.5 rounded text-xs font-mono border transition-colors ${
              selectedDecade === null
                ? 'border-neutral-900 dark:border-neutral-100 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-semibold shadow-xs'
                : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 text-neutral-600 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-700'
            }`}
          >
            All
          </button>
          {decades.map((dec) => {
            const isSelected = dec === selectedDecade;
            return (
              <button
                key={dec}
                type="button"
                onClick={() => handleSelectDecade(dec)}
                className={`px-2 py-0.5 rounded text-xs font-mono border transition-colors ${
                  isSelected
                    ? 'border-neutral-900 dark:border-neutral-100 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-semibold shadow-xs'
                    : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 text-neutral-600 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-700'
                }`}
              >
                {dec}s
              </button>
            );
          })}
        </div>

        <button
          type="button"
          onClick={handleSelectToday}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-xs font-medium border border-neutral-200 dark:border-neutral-700 bg-neutral-50 dark:bg-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-700 text-neutral-800 dark:text-neutral-200 transition-colors shrink-0"
        >
          <span className="w-1.5 h-1.5 rounded-full bg-blue-500" />
          <span>Open today</span>
        </button>
      </div>

      {/* Grid Canvas */}
      <div
        ref={containerRef}
        className="w-full relative cursor-pointer rounded-md border border-neutral-200 dark:border-neutral-800 bg-neutral-50/40 dark:bg-neutral-950/40 select-none"
      >
        <canvas
          ref={canvasRef}
          onMouseEnter={handlePointerEnter}
          onMouseMove={(e) => handlePointerMove(e.clientX, e.clientY)}
          onMouseLeave={handlePointerLeave}
          onTouchStart={handlePointerEnter}
          onTouchMove={(e) => e.touches[0] && handlePointerMove(e.touches[0].clientX, e.touches[0].clientY)}
          onTouchEnd={handlePointerLeave}
          onClick={handleClick}
          className="block w-full rounded-md"
        />

        {/* Floating Tooltip HUD */}
        <div
          ref={tooltipRef}
          style={{ display: 'none' }}
          className="pointer-events-none absolute left-0 top-0 z-20 px-3 py-2 rounded-lg text-xs font-medium shadow-xl bg-neutral-900/95 text-neutral-100 dark:bg-neutral-100/95 dark:text-neutral-900 border border-neutral-700/80 dark:border-neutral-300/80 whitespace-nowrap will-change-transform"
        >
          <div className="flex items-center gap-2">
            <span ref={tooltipDateRef} className="font-mono font-bold tracking-tight text-sm"></span>
            <span ref={tooltipBadgeRef}></span>
            <span
              ref={tooltipTimeRef}
              style={{ display: 'none' }}
              className="text-[11px] font-mono text-neutral-300 dark:text-neutral-600 ml-0.5"
            ></span>
          </div>
        </div>
      </div>
    </div>
  );
};
