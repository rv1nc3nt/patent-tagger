<script lang="ts">
  import { drawingPageUrl, type DrawingPage } from "./api";
  import RotatedImage from "./RotatedImage.svelte";

  // The View screen's figure viewer: one page at a time, fitted to the
  // panel width, with zoom, rotation and a strip of thumbnails.
  let {
    docId,
    pages,
    current = $bindable(0),
    onRotate,
  }: {
    docId: number;
    /// Real pages only (page >= 1), in page order.
    pages: DrawingPage[];
    /// Index into `pages`.
    current?: number;
    onRotate: (page: DrawingPage, rotation: number) => void;
  } = $props();

  let urls = $state<Record<number, string>>({});
  let zoom = $state(1);
  let viewportWidth = $state(0);

  // Object URLs for every page of the document, revoked when it changes.
  $effect(() => {
    const created: string[] = [];
    let cancelled = false;
    urls = {};
    for (const p of pages) {
      drawingPageUrl(docId, p.page).then((u) => {
        if (cancelled) {
          URL.revokeObjectURL(u);
          return;
        }
        created.push(u);
        urls[p.page] = u;
      });
    }
    return () => {
      cancelled = true;
      created.forEach((u) => URL.revokeObjectURL(u));
    };
  });

  const page = $derived(pages[Math.min(current, pages.length - 1)]);

  export function step(delta: number) {
    if (pages.length === 0) return;
    current = (current + delta + pages.length) % pages.length;
  }

  export function turn(delta: number) {
    if (page) onRotate(page, (page.rotation + delta + 360) % 360);
  }

  function setZoom(next: number) {
    zoom = Math.max(0.5, Math.min(4, Math.round(next * 100) / 100));
  }
</script>

{#if page}
  <div class="figures">
    <div class="controls">
      <button onclick={() => step(-1)} disabled={pages.length < 2} title="Previous page ([)">‹</button>
      <span class="page-label">Page {page.page} / {pages.length}</span>
      <button onclick={() => step(1)} disabled={pages.length < 2} title="Next page (])">›</button>
      <span class="spacer"></span>
      <button onclick={() => turn(-90)} title="Rotate left (Shift+R)" aria-label="Rotate left">↺</button>
      <button onclick={() => turn(90)} title="Rotate right (R)" aria-label="Rotate right">↻</button>
      <span class="spacer"></span>
      <button onclick={() => setZoom(zoom / 1.25)} disabled={zoom <= 0.5} title="Zoom out (-)" aria-label="Zoom out">−</button>
      <button class="zoom-label" onclick={() => setZoom(1)} title="Fit to panel (0)">{Math.round(zoom * 100)}%</button>
      <button onclick={() => setZoom(zoom * 1.25)} disabled={zoom >= 4} title="Zoom in (+)" aria-label="Zoom in">+</button>
    </div>

    <div class="viewport" bind:clientWidth={viewportWidth}>
      {#if urls[page.page] && viewportWidth > 0}
        <RotatedImage
          src={urls[page.page]}
          alt="Drawing page {page.page}"
          rotation={page.rotation}
          width={(viewportWidth - 2) * zoom}
          naturalWidth={page.width}
          naturalHeight={page.height}
        />
      {:else}
        <p class="note">Loading…</p>
      {/if}
    </div>

    {#if pages.length > 1}
      <div class="strip">
        {#each pages as p, i (p.page)}
          <button class="thumb" class:current={i === current} onclick={() => (current = i)} aria-label="Page {p.page}">
            {#if urls[p.page]}
              <img src={urls[p.page]} alt="" style:transform="rotate({p.rotation}deg)" />
            {/if}
            <span class="page-number">{p.page}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .figures {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    height: 100%;
    min-height: 0;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    flex-wrap: wrap;
  }
  .controls button {
    padding: 0.15rem 0.5rem;
    min-width: 1.9rem;
  }
  .page-label {
    font-size: 0.85rem;
    font-variant-numeric: tabular-nums;
    padding: 0 0.25rem;
  }
  .zoom-label {
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
    min-width: 3.4rem !important;
  }
  .spacer {
    flex: 1;
  }
  .viewport {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-alt);
  }
  .strip {
    display: flex;
    gap: 0.35rem;
    overflow-x: auto;
    padding-bottom: 0.25rem;
    flex: none;
  }
  .thumb {
    position: relative;
    flex: none;
    width: 56px;
    height: 56px;
    padding: 0;
    overflow: hidden;
    background: white;
    border: 2px solid var(--border);
    border-radius: 4px;
  }
  .thumb.current {
    border-color: var(--accent);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .page-number {
    position: absolute;
    bottom: 1px;
    right: 3px;
    font-size: 0.65rem;
    color: #444;
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
    padding: 0.5rem;
  }
</style>
