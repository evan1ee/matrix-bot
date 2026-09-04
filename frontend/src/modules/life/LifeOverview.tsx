import React, { useEffect, useRef, useState, useCallback } from 'react';
import type { LifeHorizonDay } from './types';
import { MOOD_CONFIG } from './types';

interface LifeOverviewProps {
  days: LifeHorizonDay[];
  isDark?: boolean;
}

export const LifeOverview: React.FC<LifeOverviewProps> = ({
  days,
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

  const animFrameRef = useRef<number | null>(null);
  const hoveredCellRef = useRef<{ col: number | null; row: number | null }>({ col: null, row: null });

  const currentColsRef = useRef(100);
  const currentRowsRef = useRef(100);
  const currentCellWidthRef = useRef(4);
  const currentCellHeightRef = useRef(4);
  const currentWRef = useRef(0);
  const currentHRef = useRef(0);

  // 1. Pre-render full lifespan base grid into offscreen canvas
  const renderBaseGrid = useCallback(
    (cols: number, rows: number, cellWidth: number, cellHeight: number, w: number, h: number) => {
      if (days.length === 0 || w <= 0 || h <= 0) return;

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

      // Theme colors
      const emptyPastColor = isDark ? '#27272a' : '#e4e4e7';
      const futureColor = isDark ? '#141417' : '#fafafa';
      const futureBorderColor = isDark ? '#2e2e33' : '#e4e4e7';
      const presentColor = '#10b981';

      const total = days.length;
      const pad = Math.max(0.3, Math.min(1.0, cellWidth * 0.18));
      const bw = Math.max(1, cellWidth - pad);
      const bh = Math.max(1, cellHeight - pad);

      for (let i = 0; i < total; i++) {
        const d = days[i];
        const col = i % cols;
        const row = Math.floor(i / cols);

        const x = col * cellWidth;
        const y = row * cellHeight;

        if (d.status === 'present') {
          ctx.fillStyle = presentColor;
          ctx.fillRect(x, y, bw, bh);
          ctx.strokeStyle = '#34d399';
          ctx.lineWidth = 1;
          ctx.strokeRect(x, y, bw, bh);
        } else if (d.status === 'past_logged' && d.mood) {
          const moodKey = d.mood.toLowerCase() as keyof typeof MOOD_CONFIG;
          ctx.fillStyle = MOOD_CONFIG[moodKey]?.color || '#10b981';
          ctx.fillRect(x, y, bw, bh);
        } else if (d.status === 'past_empty') {
          ctx.fillStyle = emptyPastColor;
          ctx.fillRect(x, y, bw, bh);
        } else {
          ctx.fillStyle = futureColor;
          ctx.fillRect(x, y, bw, bh);
          if (bw > 2) {
            ctx.strokeStyle = futureBorderColor;
            ctx.lineWidth = 0.5;
            ctx.strokeRect(x, y, bw, bh);
          }
        }
      }

      ctx.restore();
    },
    [days, isDark]
  );

  // 2. Render Scene: Blit base grid & draw hover highlight
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

      // Blit pre-rendered base grid
      ctx.drawImage(baseCanvas, 0, 0, w, h);

      // Hover ring
      if (hoveredCol !== null && hoveredRow !== null) {
        const cols = currentColsRef.current;
        const idx = hoveredRow * cols + hoveredCol;
        if (idx >= 0 && idx < days.length) {
          const cellWidth = currentCellWidthRef.current;
          const cellHeight = currentCellHeightRef.current;
          const pad = 0.5;
          const bx = hoveredCol * cellWidth + pad;
          const by = hoveredRow * cellHeight + pad;
          const bw = cellWidth - pad * 2;
          const bh = cellHeight - pad * 2;

          ctx.strokeStyle = isDark ? '#ffffff' : '#09090b';
          ctx.lineWidth = 1.2;
          ctx.strokeRect(bx, by, bw, bh);
        }
      }

      ctx.restore();
    },
    [days.length, isDark]
  );

  // 3. Compact overview sizing (~150px height)
  const updateSize = useCallback(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas || days.length === 0) return;

    const containerRect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.floor(containerRect.width);
    if (w <= 0) return;

    const h = Math.min(180, Math.max(130, Math.floor(w * 0.16)));
    const total = days.length;

    const cols = Math.ceil(Math.sqrt(total * (w / h)));
    const rows = Math.ceil(total / cols);
    const cellWidth = w / cols;
    const cellHeight = h / rows;

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
  }, [days.length, renderBaseGrid, renderScene]);

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
        el.className = 'px-1.5 py-0.5 rounded text-[10px] font-bold bg-emerald-500 text-white';
        el.style.backgroundColor = '';
        el.textContent = 'TODAY';
      } else if (d.mood) {
        const moodKey = d.mood.toLowerCase() as keyof typeof MOOD_CONFIG;
        const conf = MOOD_CONFIG[moodKey];
        el.className = 'px-1.5 py-0.5 rounded text-[10px] font-bold text-white shadow-xs capitalize';
        el.style.backgroundColor = conf?.color || '#10b981';
        el.textContent = `${conf?.emoji ?? '✨'} ${d.mood}`;
      } else if (d.status === 'past_empty') {
        el.className = 'px-1.5 py-0.5 rounded text-[10px] bg-zinc-700 dark:bg-zinc-300 text-zinc-200 dark:text-zinc-800';
        el.style.backgroundColor = '';
        el.textContent = 'Unrecorded';
      } else {
        el.className = 'px-1.5 py-0.5 rounded text-[10px] bg-zinc-800 dark:bg-zinc-200 text-zinc-300 dark:text-zinc-700';
        el.style.backgroundColor = '';
        el.textContent = 'Future';
      }
    }
  }, []);

  const handlePointerMove = (clientX: number, clientY: number) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

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
    if (idx >= 0 && idx < days.length) {
      hoveredCellRef.current = { col, row };

      if (tooltipRef.current) {
        const tooltipHalfWidth = 140;
        const left = Math.max(
          tooltipHalfWidth + 10,
          Math.min(currentWRef.current - tooltipHalfWidth - 10, mouseX)
        );
        const isNearTop = mouseY < 65;
        const top = isNearTop ? mouseY + 20 : mouseY - 10;

        tooltipRef.current.style.display = 'block';
        tooltipRef.current.style.transform = `translate3d(${left}px, ${top}px, 0) translateY(${isNearTop ? '0%' : '-100%'}) translateX(-50%)`;
      }

      updateTooltipDOM(days[idx]);

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

  if (days.length === 0) return null;

  return (
    <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-5 space-y-3">
      {/* Header & Controls */}
      <div className="flex flex-col md:flex-row md:items-baseline justify-between gap-2 pb-2 border-b border-zinc-100 dark:border-zinc-800">
        <div className="flex items-center gap-2">
          <h2 className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
            Lifespan overview
          </h2>
        </div>

        {/* Compact Legend */}
        <div className="flex flex-wrap items-center gap-3 text-xs text-zinc-500 dark:text-zinc-400">
          {[
            { label: 'Past', cls: 'bg-zinc-200 dark:bg-zinc-800' },
            { label: 'Today', cls: 'bg-emerald-500 ring-1 ring-emerald-400' },
            { label: 'Logged', cls: 'bg-amber-400 dark:bg-amber-500' },
            { label: 'Future', cls: 'bg-white dark:bg-zinc-950 border border-zinc-200 dark:border-zinc-800' },
          ].map((item) => (
            <div key={item.label} className="flex items-center gap-1.5">
              <span className={`w-2.5 h-2.5 rounded-xs ${item.cls}`} />
              <span>{item.label}</span>
            </div>
          ))}
        </div>
      </div>

      <p className="text-xs text-zinc-500 dark:text-zinc-400">
        Full {days.length.toLocaleString()} days visual map. Inspect your complete life journey at a glance.
      </p>

      {/* Overview Canvas */}
      <div
        ref={containerRef}
        className="w-full relative cursor-default rounded-md border border-zinc-200 dark:border-zinc-800 bg-zinc-50/40 dark:bg-zinc-950/40 select-none overflow-hidden"
      >
        <canvas
          ref={canvasRef}
          onMouseMove={(e) => handlePointerMove(e.clientX, e.clientY)}
          onMouseLeave={handlePointerLeave}
          onTouchMove={(e) => e.touches[0] && handlePointerMove(e.touches[0].clientX, e.touches[0].clientY)}
          onTouchEnd={handlePointerLeave}
          className="block w-full rounded-md"
        />

        {/* Floating Tooltip HUD */}
        <div
          ref={tooltipRef}
          style={{ display: 'none' }}
          className="pointer-events-none absolute left-0 top-0 z-20 px-3 py-2 rounded-lg text-xs font-medium shadow-xl bg-zinc-900/95 text-zinc-100 dark:bg-zinc-100/95 dark:text-zinc-900 border border-zinc-700/80 dark:border-zinc-300/80 backdrop-blur-sm whitespace-nowrap will-change-transform"
        >
          <div className="flex items-center gap-2">
            <span ref={tooltipDateRef} className="font-mono font-bold tracking-tight text-sm"></span>
            <span ref={tooltipBadgeRef}></span>
          </div>
        </div>
      </div>
    </div>
  );
};
