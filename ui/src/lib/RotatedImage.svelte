<script lang="ts">
  // An image turned by a multiple of 90 degrees, laid out at its turned
  // size: `width` is the displayed width after rotation. A CSS rotation
  // alone would keep the unrotated box and overlap its surroundings.
  let {
    src,
    alt,
    rotation,
    width,
    naturalWidth = null,
    naturalHeight = null,
  }: {
    src: string;
    alt: string;
    rotation: number;
    width: number;
    naturalWidth?: number | null;
    naturalHeight?: number | null;
  } = $props();

  // Falls back to the loaded image's size when the stored one is unknown.
  let loaded = $state<{ w: number; h: number } | null>(null);
  const w = $derived(naturalWidth ?? loaded?.w ?? 1);
  const h = $derived(naturalHeight ?? loaded?.h ?? 1);
  const quarter = $derived(rotation % 180 !== 0);
  const boxW = $derived(width);
  const boxH = $derived(quarter ? (width * w) / h : (width * h) / w);
</script>

<div class="box" style:width="{boxW}px" style:height="{boxH}px">
  <img
    {src}
    {alt}
    style:width="{quarter ? boxH : boxW}px"
    style:height="{quarter ? boxW : boxH}px"
    style:transform="translate(-50%, -50%) rotate({rotation}deg)"
    onload={(e) => {
      const img = e.currentTarget as HTMLImageElement;
      loaded = { w: img.naturalWidth, h: img.naturalHeight };
    }}
  />
</div>

<style>
  .box {
    position: relative;
    flex: none;
  }
  img {
    position: absolute;
    left: 50%;
    top: 50%;
    background: white;
    transition: transform 0.15s;
  }
</style>
