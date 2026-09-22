<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    documentDetail,
    reviewQueue,
    skipDocument,
    validateDocument,
    type DocumentView,
    type QueueEntry,
    type QueueOrdering,
  } from "./api";
  import TagsPanel from "./TagsPanel.svelte";

  let queue = $state<QueueEntry[]>([]);
  let index = $state(0);
  let doc = $state<DocumentView | null>(null);
  let checked = $state<Set<number>>(new Set());
  let tagFilter = $state("");
  let error = $state("");
  let notice = $state("");
  let ordering = $state<QueueOrdering>("import");
  let filterInput = $state<HTMLInputElement | undefined>(undefined);

  const visibleTags = $derived(
    doc?.tags.filter((t) => t.name.toLowerCase().includes(tagFilter.toLowerCase())) ?? [],
  );

  export async function refreshQueue() {
    error = "";
    try {
      queue = await reviewQueue(ordering);
      if (index >= queue.length) index = Math.max(0, queue.length - 1);
      await loadCurrent();
    } catch (err) {
      error = String(err);
    }
  }

  function handleOrderingChange(next: QueueOrdering) {
    ordering = next;
    index = 0;
    refreshQueue();
  }

  async function loadCurrent() {
    if (queue.length === 0) {
      doc = null;
      return;
    }
    try {
      doc = await documentDetail(queue[index].id);
      checked = new Set(doc?.tags.filter((t) => t.suggested).map((t) => t.tag_id) ?? []);
    } catch (err) {
      error = String(err);
    }
  }

  function next() {
    if (index < queue.length - 1) {
      index += 1;
      loadCurrent();
    }
  }

  function prev() {
    if (index > 0) {
      index -= 1;
      loadCurrent();
    }
  }

  async function advanceAfterRemoval() {
    queue = queue.filter((_, i) => i !== index);
    if (index >= queue.length) index = Math.max(0, queue.length - 1);
    await loadCurrent();
  }

  async function handleValidate() {
    if (!doc) return;
    error = "";
    try {
      const result = await validateDocument(doc.id, Array.from(checked));
      notice =
        result.auto_disabled_tags.length > 0
          ? `Automatic mode disabled (audited precision dropped below target) for: ${result.auto_disabled_tags.join(", ")}`
          : "";
      await advanceAfterRemoval();
    } catch (err) {
      error = String(err);
    }
  }

  async function handleSkip() {
    if (!doc) return;
    error = "";
    try {
      await skipDocument(doc.id);
      await advanceAfterRemoval();
    } catch (err) {
      error = String(err);
    }
  }

  function toggleTag(tagId: number) {
    const next = new Set(checked);
    if (next.has(tagId)) next.delete(tagId);
    else next.add(tagId);
    checked = next;
  }

  function espacenetUrl(d: DocumentView): string | null {
    const match = d.pub_key.match(/^([A-Za-z]+)(\d+)$/);
    if (!match) return null;
    const [, country, number] = match;
    const kind = d.kind_codes[0] ?? "";
    return `https://worldwide.espacenet.com/publicationDetails/biblio?CC=${country}&NR=${number}${kind}&KC=${kind}&FT=D`;
  }

  const espacenetLink = $derived(doc ? espacenetUrl(doc) : null);

  function handleKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const isTyping = target?.tagName === "INPUT" || target?.tagName === "TEXTAREA";

    if (e.key === "/") {
      e.preventDefault();
      filterInput?.focus();
      return;
    }
    if (isTyping || !doc) return;

    if (e.key === "j" || e.key === "J") {
      next();
    } else if (e.key === "k" || e.key === "K") {
      prev();
    } else if (e.key === "Enter") {
      e.preventDefault();
      handleValidate();
    } else if (e.key === "s" || e.key === "S") {
      handleSkip();
    } else {
      const tag = doc.tags.find((t) => t.hotkey?.toLowerCase() === e.key.toLowerCase());
      if (tag) toggleTag(tag.tag_id);
    }
  }

  $effect(() => {
    refreshQueue();
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="review">
  <TagsPanel onchange={refreshQueue} />

  {#if error}<p class="error-banner">{error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if queue.length === 0}
    <p class="empty">Nothing to review. Import and fetch some documents first.</p>
  {:else}
    <div class="panes">
      <section class="queue-pane">
        <div class="queue-header">
          <h2>Queue ({index + 1} / {queue.length})</h2>
          <select value={ordering} onchange={(e) => handleOrderingChange(e.currentTarget.value as QueueOrdering)}>
            <option value="import">Import order</option>
            <option value="uncertain">Most uncertain first</option>
          </select>
        </div>
        <ul>
          {#each queue as entry, i (entry.id)}
            <li class:current={i === index}>
              <button
                onclick={() => {
                  index = i;
                  loadCurrent();
                }}
              >
                {entry.pub_key} — {entry.title ?? "…"}
              </button>
            </li>
          {/each}
        </ul>
      </section>

      {#if doc}
        <section class="detail-pane">
          <h2>{doc.pub_key}</h2>
          <p class="title">{doc.title}</p>
          <p class="meta">
            {doc.applicants.join("; ")}
            {#if doc.publication_date}· {doc.publication_date}{/if}
          </p>
          {#if doc.cpc.length > 0}<p class="meta">CPC: {doc.cpc.join(", ")}</p>{/if}
          {#if espacenetLink}
            <button class="link" onclick={() => openUrl(espacenetLink)}>Open in Espacenet</button>
          {/if}
          <p class="abstract">{doc.abstract_text}</p>
        </section>

        <section class="tags-pane">
          <h2>Tags</h2>
          <input
            class="filter"
            placeholder="Filter tags (/)"
            bind:value={tagFilter}
            bind:this={filterInput}
          />
          <ul>
            {#each visibleTags as tag (tag.tag_id)}
              <li class:weak={tag.source === "zero_shot"}>
                <label>
                  <input
                    type="checkbox"
                    checked={checked.has(tag.tag_id)}
                    onchange={() => toggleTag(tag.tag_id)}
                  />
                  <span class="name">{tag.name}</span>
                  {#if tag.hotkey}<kbd>{tag.hotkey}</kbd>{/if}
                </label>
                {#if tag.score !== null}
                  <div class="score-bar">
                    <div class="fill" style:width="{tag.score * 100}%"></div>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>

    <div class="shortcuts">
      <kbd>J</kbd>/<kbd>K</kbd> next/prev · <kbd>Enter</kbd> validate · <kbd>S</kbd> skip · <kbd>/</kbd> filter
      tags · tag hotkeys toggle
    </div>
  {/if}
</div>

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1.5rem;
    max-width: 1100px;
    margin: 0 auto;
  }
  .error-banner {
    color: var(--danger);
  }
  .notice {
    color: var(--series-recall);
  }
  .empty {
    color: var(--muted);
  }
  .queue-header {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 0.5rem;
  }
  .queue-header h2 {
    margin: 0;
  }
  .queue-header select {
    font-size: 0.8rem;
    padding: 0.2rem;
  }
  .panes {
    display: grid;
    grid-template-columns: 220px 1fr 260px;
    gap: 1rem;
    align-items: start;
  }
  .queue-pane ul,
  .tags-pane ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .queue-pane button {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 0.25rem 0.4rem;
    font-size: 0.85rem;
  }
  .queue-pane li.current button {
    background: var(--bg-alt);
    border-radius: 4px;
    font-weight: 600;
  }
  .detail-pane .title {
    font-weight: 600;
  }
  .meta {
    color: var(--muted);
    font-size: 0.85rem;
    margin: 0.25rem 0;
  }
  .abstract {
    margin-top: 0.75rem;
    line-height: 1.5;
    max-height: 40vh;
    overflow-y: auto;
  }
  .link {
    font-size: 0.8rem;
  }
  .tags-pane .filter {
    width: 100%;
    padding: 0.3rem 0.5rem;
    margin-bottom: 0.5rem;
  }
  .tags-pane li {
    padding: 0.25rem 0;
    border-bottom: 1px solid var(--border);
  }
  .tags-pane li.weak label {
    opacity: 0.7;
    font-style: italic;
  }
  .tags-pane label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .tags-pane .name {
    flex: 1;
  }
  .score-bar {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    margin-top: 0.2rem;
    overflow: hidden;
  }
  .score-bar .fill {
    height: 100%;
    background: var(--accent);
  }
  kbd {
    font-family: var(--mono);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 0.3rem;
    font-size: 0.75rem;
  }
  .shortcuts {
    color: var(--muted);
    font-size: 0.8rem;
  }
</style>
