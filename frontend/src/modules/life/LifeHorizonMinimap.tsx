import React, { useEffect, useRef, useState, useCallback } from 'react';
import type { LifeHorizonDay } from './types';
import { MOOD_CONFIG } from './types';

interface LifeHorizonMinimapProps {
  days: LifeHorizonDay[];
  currentYear: number;
  onSelectYear: (year: number) => void;
  isDark?: boolean;
}

export const LifeHorizonMinimap: React.FC<LifeHorizonMinimapProps> = ({
  days,
  currentYear,
  onSelectYear,
  isDark = true,
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [hoveredDay, setHoveredDay] = useState<{ date: string; year: number; status: string; mood: string | null } | null>(null);
  const [mousePos, setMousePos] = useState<{ x: number; y: number } | null>(null);

  // Layout parameters for canvas
  const drawMinimap = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || days.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const width = canvas.width;
    const height = canvas.height;
    ctx.clearRect(0, 0, width, height);

    // Color palette based on theme
    const emptyPastColor = isDark ? '#27272a' : '#e4e4e7'; // zinc-800 or zinc-200
    const futureColor = isDark ? '#18181b' : '#f4f4f5'; // zinc-900 or zinc-100
    const futureBorderColor = isDark ? '#3f3f46' : '#d4d4d8';
    const presentColor = '#10b981'; // emerald-500

    const total = days.length;
    // Calculate optimal grid: e.g., cols based on canvas aspect ratio
    // Target roughly width:height ~ 4:1 to 6:1
    const cols = Math.ceil(Math.sqrt(total * (width / height)));
    const rows = Math.ceil(total / cols);

    const cellWidth = width / cols;
    const cellHeight = height / rows;
    const pad = Math.max(0.2, Math.min(1, cellWidth * 0.2));

    let selectedYearMinX = Infinity;
    let selectedYearMaxX = -Infinity;
    let selectedYearMinY = Infinity;
    let selectedYearMaxY = -Infinity;

    for (let i = 0; i < total; i++) {
      const d = days[i];
      const col = i % cols;
      const row = Math.floor(i / cols);

      const x = col * cellWidth;
      const y = row * cellHeight;
      const w = Math.max(1, cellWidth - pad);
      const h = Math.max(1, cellHeight - pad);

      if (d.year === currentYear) {
        selectedYearMinX = Math.min(selectedYearMinX, x);
        selectedYearMaxX = Math.max(selectedYearMaxX, x + w);
        selectedYearMinY = Math.min(selectedYearMinY, y);
        selectedYearMaxY = Math.max(selectedYearMaxY, y + h);
      }

      if (d.status === 'present') {
        ctx.fillStyle = presentColor;
        ctx.fillRect(x, y, w, h);
      } else if (d.status === 'past_logged' && d.mood) {
        const moodKey = d.mood.toLowerCase() as keyof typeof MOOD_CONFIG;
        ctx.fillStyle = MOOD_CONFIG[moodKey]?.color || '#10b981';
        ctx.fillRect(x, y, w, h);
      } else if (d.status === 'past_empty') {
        ctx.fillStyle = emptyPastColor;
        ctx.fillRect(x, y, w, h);
      } else {
        // Future
        ctx.fillStyle = futureColor;
        ctx.fillRect(x, y, w, h);
        if (cellWidth > 3) {
          ctx.strokeStyle = futureBorderColor;
          ctx.lineWidth = 0.5;
          ctx.strokeRect(x, y, w, h);
        }
      }
    }

    // Highlight selected year region
    if (selectedYearMinX < Infinity) {
      ctx.strokeStyle = isDark ? '#38bdf8' : '#0284c7'; // cyan / sky
      ctx.lineWidth = 2;
      ctx.strokeRect(
        selectedYearMinX - 1,
        selectedYearMinY - 1,
        selectedYearMaxX - selectedYearMinX + 2,
        selectedYearMaxY - selectedYearMinY + 2
      );
    }
  }, [days, currentYear, isDark]);

  // Resize canvas according to container
  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas) return;

    const updateSize = () => {
      const rect = container.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const w = Math.floor(rect.width);
      const h = Math.min(180, Math.max(110, Math.floor(w * 0.16)));

      canvas.width = w * dpr;
      canvas.height = h * dpr;
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;

      const ctx = canvas.getContext('2d');
      if (ctx) {
        ctx.scale(dpr, dpr);
      }
      drawMinimap();
    };

    updateSize();
    const ro = new ResizeObserver(updateSize);
    ro.observe(container);

    return () => ro.disconnect();
  }, [drawMinimap]);

  // Click & hover tracking
  const getDayAtCoords = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas || days.length === 0) return null;

    const rect = canvas.getBoundingClientRect();
    const mouseX = e.clientX - rect.left;
    const mouseY = e.clientY - rect.top;

    const total = days.length;
    const cols = Math.ceil(Math.sqrt(total * (rect.width / rect.height)));
    const rows = Math.ceil(total / cols);

    const cellWidth = rect.width / cols;
    const cellHeight = rect.height / rows;

    const col = Math.floor(mouseX / cellWidth);
    const row = Math.floor(mouseY / cellHeight);

    const index = row * cols + col;
    if (index >= 0 && index < total) {
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
      onSelectYear(info.day.year);
    }
  };

  if (days.length === 0) return null;

  return (
    <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800/80 rounded-xl p-4 shadow-sm">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 mb-3">
        <div>
          <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100 flex items-center gap-2">
            <span>Life Horizon</span>
            <span className="text-xs font-normal text-zinc-500">
              (~{days.length.toLocaleString()} days macro overview)
            </span>
          </h3>
          <p className="text-xs text-zinc-400 mt-0.5">
            Click anywhere on the timeline to jump to that year. Blue outline marks Year {currentYear}.
          </p>
        </div>

        {/* Legend */}
        <div className="flex flex-wrap items-center gap-3 text-[11px] text-zinc-500 dark:text-zinc-400">
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-sm bg-zinc-300 dark:bg-zinc-700" />
            <span>Past</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-sm bg-emerald-500" />
            <span>Today</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-sm bg-amber-400" />
            <span>Logged</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-sm border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-900" />
            <span>Future</span>
          </div>
        </div>
      </div>

      <div ref={containerRef} className="w-full relative cursor-crosshair overflow-hidden rounded-lg">
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
            <div className="text-[10px] opacity-75">Click to view year</div>
          </div>
        )}
      </div>
    </div>
  );
};
