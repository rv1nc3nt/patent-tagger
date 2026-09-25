<script lang="ts" module>
  export interface FindMatch {
    section: string;
    start: number;
    end: number;
    index: number;
  }
</script>

<script lang="ts">
  import type { Annotation, ClaimRef } from "./api";

  // One block (`start..end`) of a section's `text`. Every span carries its
  // offset in the section (`data-offset`), so that the View screen can map
  // a selection back to the stored text. The spans are written without
  // whitespace between them, so the rendered text is exactly the slice.
  let {
    text,
    start,
    end,
    annotations,
    matches,
    currentMatch,
    claimRefs = [],
    onClaim,
    onAnnotation,
  }: {
    text: string;
    start: number;
    end: number;
    annotations: Annotation[];
    matches: FindMatch[];
    currentMatch: number;
    claimRefs?: ClaimRef[];
    onClaim?: (claim: number) => void;
    onAnnotation?: (id: number, anchor: DOMRect) => void;
  } = $props();

  interface Segment {
    start: number;
    end: number;
    highlights: Annotation[];
    match: FindMatch | undefined;
    claimRef: ClaimRef | undefined;
  }

  const segments = $derived.by(() => {
    const overlaps = <T extends { start: number; end: number }>(items: T[]) =>
      items.filter((i) => i.end > start && i.start < end);
    const hls = overlaps(annotations);
    const found = overlaps(matches);
    const refs = overlaps(claimRefs);

    const points = new Set([start, end]);
    for (const item of [...hls, ...found, ...refs]) {
      points.add(Math.max(start, item.start));
      points.add(Math.min(end, item.end));
    }
    const sorted = [...points].sort((a, b) => a - b);
    const out: Segment[] = [];
    for (let i = 0; i + 1 < sorted.length; i++) {
      const s = sorted[i];
      const e = sorted[i + 1];
      const covers = (item: { start: number; end: number }) => item.start <= s && item.end >= e;
      out.push({
        start: s,
        end: e,
        highlights: hls.filter(covers),
        match: found.find(covers),
        claimRef: refs.find(covers),
      });
    }
    return out;
  });

  function selectionIsEmpty(): boolean {
    const sel = window.getSelection();
    return !sel || sel.isCollapsed;
  }

  function handleClick(seg: Segment, e: MouseEvent) {
    // A drag that selects text also ends in a click; leave it to the
    // selection toolbar.
    if (!selectionIsEmpty()) return;
    if (seg.claimRef && onClaim) {
      onClaim(seg.claimRef.claim);
    } else if (seg.highlights.length > 0 && onAnnotation) {
      const top = seg.highlights[seg.highlights.length - 1];
      onAnnotation(top.id, (e.currentTarget as HTMLElement).getBoundingClientRect());
    }
  }

  function handleKey(seg: Segment, e: KeyboardEvent) {
    if (e.key === "Enter" && seg.claimRef && onClaim) onClaim(seg.claimRef.claim);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
{#each segments as seg (seg.start)}{#if seg.claimRef || seg.highlights.length > 0}<span
      data-offset={seg.start}
      data-hl={seg.highlights.map((h) => h.id).join(" ")}
      data-match={seg.match?.index}
      class:hl={seg.highlights.length > 0}
      class:commented={seg.highlights.some((h) => h.comment)}
      class:match={seg.match}
      class:current={seg.match?.index === currentMatch}
      class:claim-ref={seg.claimRef}
      style:--hl-color={seg.highlights.length > 0
        ? `var(--hl-${seg.highlights[seg.highlights.length - 1].color})`
        : undefined}
      role={seg.claimRef ? "link" : "button"}
      tabindex={seg.claimRef ? 0 : -1}
      title={seg.claimRef ? `Go to claim ${seg.claimRef.claim}` : seg.highlights.map((h) => h.comment).filter(Boolean).join("\n")}
      onclick={(e) => handleClick(seg, e)}
      onkeydown={(e) => handleKey(seg, e)}>{text.slice(seg.start, seg.end)}</span
    >{:else}<span
      data-offset={seg.start}
      data-match={seg.match?.index}
      class:match={seg.match}
      class:current={seg.match?.index === currentMatch}>{text.slice(seg.start, seg.end)}</span
    >{/if}{/each}

<style>
  .hl {
    background: var(--hl-color);
    border-radius: 2px;
    cursor: pointer;
  }
  .hl.commented {
    border-bottom: 2px solid var(--text);
  }
  .match {
    outline: 1px solid var(--find-outline);
    background: var(--find-bg);
  }
  .match.current {
    background: var(--find-current);
    color: #000;
  }
  .claim-ref {
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
  }
</style>
