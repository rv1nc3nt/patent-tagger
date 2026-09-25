<script lang="ts">
  import { drawingPageUrl } from "./api";
  import RotatedImage from "./RotatedImage.svelte";

  let {
    docId,
    page,
    size = 140,
    rotation = 0,
    naturalWidth = null,
    naturalHeight = null,
    onRotate,
  }: {
    docId: number;
    page: number;
    size?: number;
    rotation?: number;
    naturalWidth?: number | null;
    naturalHeight?: number | null;
    /// Given, the zoomed view has rotate buttons; called with the new rotation.
    onRotate?: (rotation: number) => void;
  } = $props();

  let url = $state<string | null>(null);
  let zoomed = $state(false);
  let zoom = $state(1);

  // Width at which the whole turned page fits the zoomed view at zoom 1.
  const fitWidth = $derived.by(() => {
    const maxW = window.innerWidth * 0.85;
    const maxH = window.innerHeight * 0.75;
    if (!naturalWidth || !naturalHeight) return maxW;
    const quarter = rotation % 180 !== 0;
    const aspect = quarter ? naturalWidth / naturalHeight : naturalHeight / naturalWidth;
    return Math.min(maxW, maxH / aspect);
  });

  function turn(delta: number, e: Event) {
    e.stopPropagation();
    onRotate?.((rotation + delta + 360) % 360);
  }

  $effect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;
    drawingPageUrl(docId, page).then((u) => {
      if (cancelled) {
        URL.revokeObjectURL(u);
        return;
      }
      objectUrl = u;
      url = u;
    });
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  });
</script>

{#if url}
  <button class="thumb" style:width="{size}px" style:height="{size}px" onclick={() => (zoomed = true)} aria-label="Zoom page {page}">
    <img src={url} alt="Drawing page {page}" style:transform="rotate({rotation}deg)" />
    {#if page > 0}<span class="page-number">{page}</span>{/if}
  </button>
{:else}
  <div class="thumb loading" style:width="{size}px" style:height="{size}px">…</div>
{/if}

{#if zoomed && url}
  <div class="lightbox" role="button" tabindex="0" onclick={() => (zoomed = false)}
    onkeydown={(e) => e.key === "Escape" && (zoomed = false)}>
    <div class="lightbox-content">
      <div class="zoom-controls">
        <input type="range" min="0.5" max="4" step="0.1" bind:value={zoom} onclick={(e) => e.stopPropagation()} />
        {#if onRotate}
          <button onclick={(e) => turn(-90, e)} title="Rotate left">↺</button>
          <button onclick={(e) => turn(90, e)} title="Rotate right">↻</button>
        {/if}
        <button onclick={(e) => { e.stopPropagation(); zoomed = false; }}>Close</button>
      </div>
      <div class="zoom-viewport">
        <RotatedImage
          src={url}
          alt="Drawing page {page}, zoomed"
          {rotation}
          width={fitWidth * zoom}
          {naturalWidth}
          {naturalHeight}
        />
      </div>
    </div>
  </div>
{/if}

<style>
  .thumb {
    position: relative;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
    background: var(--bg-alt);
    cursor: zoom-in;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: white;
  }
  .thumb.loading {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
  }
  .page-number {
    position: absolute;
    bottom: 2px;
    right: 4px;
    font-size: 0.7rem;
    color: var(--muted);
    background: var(--bg);
    padding: 0 0.2rem;
    border-radius: 2px;
  }
  .lightbox {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .lightbox-content {
    background: var(--bg);
    border-radius: 6px;
    padding: 0.75rem;
    max-width: 90vw;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .zoom-controls {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .zoom-viewport {
    overflow: auto;
    max-width: 85vw;
    max-height: 80vh;
  }
</style>
