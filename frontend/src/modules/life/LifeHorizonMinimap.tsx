import React, { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import type { LifeHorizonDay, MoodType } from './types';
import { MOOD_CONFIG } from './types';

interface LifeHorizonMinimapProps {
  days: LifeHorizonDay[];
  onSelectDate: (date: string) => void;
  isDark?: boolean;
}

export const LifeHorizonMinimap: React.FC<LifeHorizonMinimapProps> = ({
  days,
  onSelectDate,
  isDark = true,
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [hoveredDay, setHoveredDay] = useState<{ date: string; year: number; status: string; mood: string | null } | null>(null);
  const [mousePos, setMousePos] = useState<{ x: number; y: number } | null>(null);

  const currentColsRef = useRef(100);
  const currentRowsRef = useRef(100);
  const currentCellWidthRef = useRef(4);
  const currentCellHeightRef = useRef(4);

  // Extract distinct decades for quick jump bar
  const decades = useMemo(() => {
    const set = new Set<number>();
    for (const d of days) {
      set.add(Math.floor(d.year / 10) * 10);
    }
    return Array.from(set).sort((a, b) => a - b);
  }, [days]);

  // Draw grid on canvas without any inner scroll
  const drawMinimap = useCallback(
    (cols: number, rows: number, cellWidth: number, cellHeight: number) => {
      const canvas = canvasRef.current;
      if (!canvas || days.length === 0) return;

      const ctx = canvas.getContext('2d');
      if (!ctx) return;

      const dpr = window.devicePixelRatio || 1;
      const w = canvas.width / dpr;
      const h = canvas.height / dpr;

      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.clearRect(0, 0, w, h);

      // Colors following Design.md
      const emptyPastColor = isDark ? '#27272a' : '#e4e4e7'; // zinc-800 or zinc-200
      const futureColor = isDark ? '#18181b' : '#fafafa'; // zinc-900 or zinc-50
      const futureBorderColor = isDark ? '#3f3f46' : '#d4d4d8'; // zinc-700 or zinc-300
      const presentColor = '#10b981'; // emerald-500

      const total = days.length;
      const pad = Math.max(0.6, Math.min(1.2, cellWidth * 0.16));

      for (let i = 0; i < total; i++) {
        const d = days[i];
        const col = i % cols;
        const row = Math.floor(i / cols);

        const x = col * cellWidth;
        const y = row * cellHeight;
        const bw = Math.max(1, cellWidth - pad);
        const bh = Math.max(1, cellHeight - pad);

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
          // Future: clean tile with subtle border
          ctx.fillStyle = futureColor;
          ctx.fillRect(x, y, bw, bh);
          ctx.strokeStyle = futureBorderColor;
          ctx.lineWidth = 0.6;
          ctx.strokeRect(x, y, bw, bh);
        }
      }

      ctx.restore();
    },
    [days, isDark]
  );

  // Resize canvas according to container width, 2x bigger blocks with NO inner scroll
  const updateSize = useCallback(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas || days.length === 0) return;

    const rect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.floor(rect.width);
    if (w <= 0) return;

    // 2x bigger blocks without scroll: height scaled proportionally to ~44% of width
    const h = Math.min(560, Math.max(360, Math.floor(w * 0.44)));

    const total = days.length;
    const cols = Math.ceil(Math.sqrt(total * (w / h)));
    const rows = Math.ceil(total / cols);
    const cellWidth = w / cols;
    const cellHeight = h / rows;

    currentColsRef.current = cols;
    currentRowsRef.current = rows;
    currentCellWidthRef.current = cellWidth;
    currentCellHeightRef.current = cellHeight;

    canvas.width = Math.floor(w * dpr);
    canvas.height = Math.floor(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;

    drawMinimap(cols, rows, cellWidth, cellHeight);
  }, [days.length, drawMinimap]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    updateSize();
    const ro = new ResizeObserver(updateSize);
    ro.observe(container);

    return () => ro.disconnect();
  }, [updateSize]);

  // Click & hover tracking
  const getDayAtCoords = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas || days.length === 0) return null;

    const rect = canvas.getBoundingClientRect();
    const mouseX = e.clientX - rect.left;
    const mouseY = e.clientY - rect.top;

    const cols = currentColsRef.current;
    const rows = currentRowsRef.current;
    const cellWidth = currentCellWidthRef.current;
    const cellHeight = currentCellHeightRef.current;

    const col = Math.floor(mouseX / cellWidth);
    const row = Math.floor(mouseY / cellHeight);

    const index = row * cols + col;
    if (index >= 0 && index < days.length && col < cols && row < rows) {
      return { day: days[index], x: mouseX, y: mouseY };
    }
    return null;
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const info = getDayAtCoords(e);
    if (info) {
      setHoveredDay(info.day);
      setMousePos({ x: info.x, y: info.y });
    } else {
      setHoveredDay(null);
      setMousePos(null);
    }
  };

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const info = getDayAtCoords(e);
    if (info) {
      onSelectDate(info.day.date);
    }
  };

  const [selectedDecade, setSelectedDecade] = useState<number | null>(null);

  const handleSelectDecade = (decade: number) => {
    setSelectedDecade((prev) => (prev === decade ? null : decade));
  };

  const handleSelectToday = () => {
    const canvas = canvasRef.current;
    if (canvas) {
      canvas.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
  };

  if (days.length === 0) return null;

  return (
    <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-5 space-y-4">
      {/* Header & Controls */}
      <div className="flex flex-col md:flex-row md:items-baseline justify-between gap-3 pb-3 border-b border-zinc-100 dark:border-zinc-800">
        <div>
          <h2 className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
            Lifespan day grid
          </h2>
          <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">
            {days.length.toLocaleString()} total days. White is future, green is today, and colors mark recorded moods.
          </p>
        </div>

        {/* Legend */}
        <div className="flex flex-wrap items-center gap-3 text-xs text-zinc-500 dark:text-zinc-400">
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-zinc-200 dark:bg-zinc-800" />
            <span>Past</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-emerald-500 ring-1 ring-emerald-400" />
            <span>Today</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-amber-400 dark:bg-amber-500" />
            <span>Amazing</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-emerald-500" />
            <span>Good</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-cyan-500" />
            <span>Neutral</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-purple-500" />
            <span>Low</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-rose-500" />
            <span>Rough</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-xs bg-white dark:bg-zinc-950 border border-zinc-200 dark:border-zinc-800" />
            <span>Future</span>
          </div>
        </div>
      </div>

      {/* Decade Quick-Jump Bar & Jump to Today */}
      <div className="flex items-center justify-between gap-2 overflow-x-auto pb-1 text-xs">
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-xs text-zinc-400 dark:text-zinc-500 mr-1">
            Jump to:
          </span>
          {decades.map((dec) => {
            const isSelected = dec === selectedDecade;
            return (
              <button
                key={dec}
                type="button"
                onClick={() => handleSelectDecade(dec)}
                className={`px-2 py-0.5 rounded text-xs font-mono border transition-colors ${
                  isSelected
                    ? 'border-zinc-900 dark:border-zinc-100 bg-zinc-900 dark:bg-zinc-100 text-white dark:text-zinc-900 font-semibold shadow-xs'
                    : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/60 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700'
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
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-xs font-medium border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 hover:bg-zinc-100 dark:hover:bg-zinc-700 text-zinc-800 dark:text-zinc-200 transition-colors shrink-0"
        >
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
          <span>Jump to today</span>
        </button>
      </div>

      {/* Grid Canvas: Full View Without Any Inner Scroll */}
      <div
        ref={containerRef}
        className="w-full relative cursor-crosshair overflow-hidden rounded-md border border-zinc-200 dark:border-zinc-800 bg-zinc-50/40 dark:bg-zinc-950/40"
      >
        <canvas
          ref={canvasRef}
          onMouseMove={handleMouseMove}
          onMouseLeave={() => {
            setHoveredDay(null);
            setMousePos(null);
          }}
          onClick={handleClick}
          className="block w-full"
        />

        {hoveredDay && mousePos && (
          <div
            className="pointer-events-none absolute z-20 px-2.5 py-1.5 rounded-md text-[11px] font-medium shadow-lg bg-zinc-900 text-zinc-100 dark:bg-zinc-100 dark:text-zinc-900 border border-zinc-700 dark:border-zinc-300 whitespace-nowrap transform -translate-x-1/2 -translate-y-full -mt-2"
            style={{ left: mousePos.x, top: mousePos.y }}
          >
            <div className="flex items-center gap-1.5">
              <span>{hoveredDay.date}</span>
              <span className="opacity-60">(Year {hoveredDay.year})</span>
              {hoveredDay.mood && (
                <span className="capitalize text-amber-400 dark:text-amber-600 font-bold">
                  • {hoveredDay.mood}
                </span>
              )}
            </div>
            <div className="text-[10px] opacity-75">Click to view or edit day</div>
          </div>
        )}
      </div>
    </div>
  );
};
