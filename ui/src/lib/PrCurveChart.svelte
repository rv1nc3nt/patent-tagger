<script lang="ts">
  import type { PrPoint } from "./api";

  let { points }: { points: PrPoint[] } = $props();

  const width = 260;
  const height = 140;
  const padding = { top: 10, right: 10, bottom: 20, left: 28 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;

  const x = (threshold: number) => padding.left + threshold * plotWidth;
  const y = (value: number) => padding.top + (1 - value) * plotHeight;

  // Breaks the path at gaps (null precision/recall - undefined at that
  // threshold, per SPEC 7.4) rather than interpolating across them.
  function pathFor(key: "precision" | "recall"): string {
    let d = "";
    let drawing = false;
    for (const p of points) {
      const value = p[key];
      if (value === null) {
        drawing = false;
        continue;
      }
      const cmd = drawing ? "L" : "M";
      d += `${cmd}${x(p.threshold).toFixed(1)},${y(value).toFixed(1)} `;
      drawing = true;
    }
    return d.trim();
  }

  let hoverIndex = $state<number | null>(null);

  function handleMove(e: MouseEvent) {
    const svg = e.currentTarget as SVGSVGElement;
    const rect = svg.getBoundingClientRect();
    const relX = ((e.clientX - rect.left) / rect.width) * width;
    const threshold = Math.max(0, Math.min(1, (relX - padding.left) / plotWidth));
    let nearest = 0;
    let nearestDist = Infinity;
    points.forEach((p, i) => {
      const dist = Math.abs(p.threshold - threshold);
      if (dist < nearestDist) {
        nearestDist = dist;
        nearest = i;
      }
    });
    hoverIndex = nearest;
  }

  const hovered = $derived(hoverIndex !== null ? points[hoverIndex] : null);
</script>

<div class="chart-wrap">
  <div class="legend">
    <span class="swatch precision"></span>Precision
    <span class="swatch recall"></span>Recall
  </div>
  <svg
    viewBox="0 0 {width} {height}"
    role="img"
    aria-label="Precision and recall vs. threshold"
    onmousemove={handleMove}
    onmouseleave={() => (hoverIndex = null)}
  >
    {#each [0, 0.25, 0.5, 0.75, 1] as tick (tick)}
      <line
        x1={padding.left}
        x2={width - padding.right}
        y1={y(tick)}
        y2={y(tick)}
        class="grid"
      />
      <text x={padding.left - 4} y={y(tick) + 3} class="axis-label" text-anchor="end">{tick}</text>
    {/each}
    <text x={padding.left} y={height - 4} class="axis-label">0</text>
    <text x={width - padding.right} y={height - 4} class="axis-label" text-anchor="end">1</text>
    <text x={(padding.left + width - padding.right) / 2} y={height - 4} class="axis-label" text-anchor="middle">
      threshold
    </text>

    <path d={pathFor("precision")} class="line precision" />
    <path d={pathFor("recall")} class="line recall" />

    {#if hovered}
      <line
        x1={x(hovered.threshold)}
        x2={x(hovered.threshold)}
        y1={padding.top}
        y2={height - padding.bottom}
        class="crosshair"
      />
    {/if}
  </svg>
  {#if hovered}
    <p class="tooltip">
      threshold {hovered.threshold.toFixed(2)} — precision {hovered.precision?.toFixed(2) ?? "—"}, recall {hovered.recall?.toFixed(
        2,
      ) ?? "—"}
    </p>
  {/if}
</div>

<style>
  .chart-wrap {
    display: inline-block;
  }
  .legend {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.75rem;
    color: var(--muted);
    margin-bottom: 0.15rem;
  }
  .swatch {
    width: 0.6rem;
    height: 0.15rem;
    display: inline-block;
    border-radius: 2px;
  }
  .swatch.precision {
    background: var(--series-precision);
  }
  .swatch.recall {
    background: var(--series-recall);
    margin-left: 0.5rem;
  }
  svg {
    width: 100%;
    max-width: 260px;
    cursor: crosshair;
  }
  .grid {
    stroke: var(--border);
    stroke-width: 1;
  }
  .axis-label {
    fill: var(--muted);
    font-size: 8px;
  }
  .line {
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .line.precision {
    stroke: var(--series-precision);
  }
  .line.recall {
    stroke: var(--series-recall);
  }
  .crosshair {
    stroke: var(--muted);
    stroke-width: 1;
    stroke-dasharray: 2 2;
  }
  .tooltip {
    font-size: 0.75rem;
    color: var(--muted);
    margin: 0.15rem 0 0;
  }
</style>
