import React, { useMemo, useState } from 'react';
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

export const KnowledgeGraphView: React.FC<KnowledgeGraphViewProps> = ({ onOpenNote }) => {
  const { data: graph, isLoading, isError } = useKnowledgeGraph(true);
  const [filter, setFilter] = useState('');
  const [zoom, setZoom] = useState(1);

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
  const origin = (GRAPH_SIZE - view) / 2;

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
          onClick={() => setZoom(1)}
          className={ICON_BUTTON}
        >
          <RotateCcw className="w-4 h-4" />
        </button>
      </div>

      <svg
        viewBox={`${origin} ${origin} ${view} ${view}`}
        role="group"
        aria-label="Knowledge graph"
        className="w-full h-[60vh] rounded-xl border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900"
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
          const open = () => onOpenNote(node.id);
          return (
            <g
              key={node.id}
              role="button"
              tabIndex={0}
              aria-label={`Open note ${node.title}`}
              data-testid={`graph-node-${node.id}`}
              onClick={open}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  open();
                }
              }}
              className={`cursor-pointer focus:outline-hidden transition-opacity duration-150 ${
                dimmed ? 'opacity-20' : 'opacity-100'
              }`}
            >
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
