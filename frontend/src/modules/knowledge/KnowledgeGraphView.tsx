import React, { useMemo, useRef, useState } from 'react';
import { Minus, Plus, RotateCcw, Search } from 'lucide-react';
import { useKnowledgeGraph } from './api';
import { GRAPH_SIZE, layoutGraph } from './graphLayout';
import { titleKey } from './wikiLinks';

interface KnowledgeGraphViewProps {
  onOpenNote: (id: number) => void;
}

const MIN_ZOOM = 1;
const MAX_ZOOM = 4;

const ICON_BUTTON =
  'p-2 rounded-md border border-neutral-200 dark:border-neutral-800 text-neutral-600 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors';

function truncate(title: string): string {
  return title.length > 22 ? `${title.slice(0, 21)}…` : title;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export const KnowledgeGraphView: React.FC<KnowledgeGraphViewProps> = ({ onOpenNote }) => {
  const { data: graph, isLoading, isError } = useKnowledgeGraph(true);
  const [filter, setFilter] = useState('');
  const [zoom, setZoom] = useState(1);
  // The point the view is centred on; dragging the canvas or focusing a node moves it.
  const [centre, setCentre] = useState({ x: GRAPH_SIZE / 2, y: GRAPH_SIZE / 2 });
  const drag = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  const positions = useMemo(() => (graph ? layoutGraph(graph) : null), [graph]);
  const linked = useMemo(() => {
    const ids = new Set<number>();
    graph?.edges.forEach((e) => {
      ids.add(e.source);
      ids.add(e.target);
    });
    return ids;
  }, [graph]);

  if (isLoading) {
    return (
      <div className="py-16 text-center text-xs text-neutral-400 font-mono">Loading graph...</div>
    );
  }
  if (isError || !graph || !positions) {
    return (
      <div role="alert" className="py-16 text-center text-xs text-rose-500">
        Could not load the knowledge graph.
      </div>
    );
  }
  if (graph.nodes.length === 0) {
    return (
      <div className="py-16 text-center rounded-xl border border-dashed border-neutral-300 dark:border-neutral-800 text-xs text-neutral-500">
        Create notes and link them with [[Note Title]] to see the graph.
      </div>
    );
  }

  const needle = titleKey(filter);
  const view = GRAPH_SIZE / zoom;
  const originX = clamp(centre.x - view / 2, 0, GRAPH_SIZE - view);
  const originY = clamp(centre.y - view / 2, 0, GRAPH_SIZE - view);

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    if (zoom === 1) return;
    drag.current = { x: e.clientX, y: e.clientY, moved: false };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent<SVGSVGElement>) => {
    const start = drag.current;
    if (!start) return;
    const unitsPerPixel = view / e.currentTarget.clientWidth;
    const dx = (e.clientX - start.x) * unitsPerPixel;
    const dy = (e.clientY - start.y) * unitsPerPixel;
    if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) > 3) start.moved = true;
    drag.current = { ...start, x: e.clientX, y: e.clientY };
    setCentre((c) => ({
      x: clamp(c.x - dx, view / 2, GRAPH_SIZE - view / 2),
      y: clamp(c.y - dy, view / 2, GRAPH_SIZE - view / 2),
    }));
  };
  const onPointerEnd = () => {
    // Keep `moved` readable for the click that follows a drag.
    setTimeout(() => {
      drag.current = null;
    }, 0);
  };

  return (
    <div className="space-y-3" data-testid="knowledge-graph">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-neutral-400" />
          <input
            type="text"
            aria-label="Filter graph nodes"
            placeholder="Highlight notes by title..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="w-full pl-9 pr-4 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-lg text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
          />
        </div>
        <button
          type="button"
          aria-label="Zoom in"
          onClick={() => setZoom((z) => Math.min(MAX_ZOOM, z * 1.5))}
          className={ICON_BUTTON}
        >
          <Plus className="w-4 h-4" />
        </button>
        <button
          type="button"
          aria-label="Zoom out"
          onClick={() => setZoom((z) => Math.max(MIN_ZOOM, z / 1.5))}
          className={ICON_BUTTON}
        >
          <Minus className="w-4 h-4" />
        </button>
        <button
          type="button"
          aria-label="Reset zoom"
          onClick={() => {
            setZoom(1);
            setCentre({ x: GRAPH_SIZE / 2, y: GRAPH_SIZE / 2 });
          }}
          className={ICON_BUTTON}
        >
          <RotateCcw className="w-4 h-4" />
        </button>
      </div>

      <svg
        viewBox={`${originX} ${originY} ${view} ${view}`}
        role="group"
        aria-label="Knowledge graph"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerEnd}
        onPointerCancel={onPointerEnd}
        className={`w-full h-[60vh] touch-none rounded-xl border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900 ${
          zoom > 1 ? 'cursor-grab' : ''
        }`}
      >
        {graph.edges.map((e) => {
          const a = positions.get(e.source);
          const b = positions.get(e.target);
          if (!a || !b) return null;
          return (
            <line
              key={`${e.source}-${e.target}`}
              data-testid={`graph-edge-${e.source}-${e.target}`}
              x1={a.x}
              y1={a.y}
              x2={b.x}
              y2={b.y}
              className="stroke-neutral-300 dark:stroke-neutral-700"
              strokeWidth={1}
            />
          );
        })}
        {graph.nodes.map((node) => {
          const p = positions.get(node.id);
          if (!p) return null;
          const dimmed = needle !== '' && !titleKey(node.title).includes(needle);
          const open = () => {
            if (drag.current?.moved) return;
            onOpenNote(node.id);
          };
          return (
            <g
              key={node.id}
              role="button"
              tabIndex={0}
              aria-label={`Open note ${node.title}`}
              data-testid={`graph-node-${node.id}`}
              onClick={open}
              // Tabbing to a node outside the zoomed view brings it into view.
              onFocus={() => setCentre({ x: p.x, y: p.y })}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  open();
                }
              }}
              className={`group cursor-pointer outline-hidden transition-opacity duration-150 ${
                dimmed ? 'opacity-20' : 'opacity-100'
              }`}
            >
              <circle
                cx={p.x}
                cy={p.y}
                r={(linked.has(node.id) ? 7 : 5) + 6}
                className="fill-none stroke-transparent group-focus-visible:stroke-neutral-900 dark:group-focus-visible:stroke-neutral-100"
                strokeWidth={2}
              />
              <circle
                cx={p.x}
                cy={p.y}
                r={linked.has(node.id) ? 7 : 5}
                className="fill-neutral-900 dark:fill-neutral-100 stroke-transparent hover:stroke-neutral-400"
                strokeWidth={3}
              />
              <text
                x={p.x}
                y={p.y + 18}
                textAnchor="middle"
                className="fill-neutral-700 dark:fill-neutral-300 font-mono"
                fontSize={11}
              >
                {truncate(node.title)}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
};
