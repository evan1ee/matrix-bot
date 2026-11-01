import type { KnowledgeGraph } from './types';

export interface Point {
  x: number;
  y: number;
}

/** Side of the square the layout is computed in. */
export const GRAPH_SIZE = 800;

/**
 * Fruchterman-Reingold layout: nodes repel, edges pull, and the system cools
 * over a fixed number of steps. Deterministic (nodes start on a circle), so a
 * graph renders the same way on every visit and in tests.
 */
export function layoutGraph(graph: KnowledgeGraph): Map<number, Point> {
  const n = graph.nodes.length;
  const positions = new Map<number, Point>();
  if (n === 0) return positions;

  const centre = GRAPH_SIZE / 2;
  const radius = GRAPH_SIZE * 0.35;
  const xs = new Float64Array(n);
  const ys = new Float64Array(n);
  const index = new Map<number, number>();
  graph.nodes.forEach((node, i) => {
    index.set(node.id, i);
    const angle = (2 * Math.PI * i) / n;
    xs[i] = centre + radius * Math.cos(angle);
    ys[i] = centre + radius * Math.sin(angle);
  });
  const edges = graph.edges
    .map((e) => [index.get(e.source), index.get(e.target)] as const)
    .filter((e): e is readonly [number, number] => e[0] !== undefined && e[1] !== undefined);

  const k = 0.8 * Math.sqrt((GRAPH_SIZE * GRAPH_SIZE) / n);
  // O(n²) per step: fewer steps for big graphs keeps the first paint quick.
  const steps = n > 200 ? 80 : 200;
  const dx = new Float64Array(n);
  const dy = new Float64Array(n);

  for (let step = 0; step < steps; step++) {
    const temperature = (GRAPH_SIZE / 10) * (1 - step / steps);
    dx.fill(0);
    dy.fill(0);
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let ex = xs[i] - xs[j];
        let ey = ys[i] - ys[j];
        const dist = Math.max(Math.hypot(ex, ey), 0.01);
        const force = (k * k) / dist;
        ex = (ex / dist) * force;
        ey = (ey / dist) * force;
        dx[i] += ex;
        dy[i] += ey;
        dx[j] -= ex;
        dy[j] -= ey;
      }
    }
    for (const [a, b] of edges) {
      const ex = xs[a] - xs[b];
      const ey = ys[a] - ys[b];
      const dist = Math.max(Math.hypot(ex, ey), 0.01);
      const force = (dist * dist) / k;
      dx[a] -= (ex / dist) * force;
      dy[a] -= (ey / dist) * force;
      dx[b] += (ex / dist) * force;
      dy[b] += (ey / dist) * force;
    }
    for (let i = 0; i < n; i++) {
      // A weak pull to the middle keeps disconnected notes on screen.
      dx[i] += (centre - xs[i]) * 0.05;
      dy[i] += (centre - ys[i]) * 0.05;
      const len = Math.max(Math.hypot(dx[i], dy[i]), 0.01);
      const move = Math.min(len, temperature);
      xs[i] = Math.min(GRAPH_SIZE - 20, Math.max(20, xs[i] + (dx[i] / len) * move));
      ys[i] = Math.min(GRAPH_SIZE - 20, Math.max(20, ys[i] + (dy[i] / len) * move));
    }
  }

  graph.nodes.forEach((node, i) => positions.set(node.id, { x: xs[i], y: ys[i] }));
  return positions;
}
