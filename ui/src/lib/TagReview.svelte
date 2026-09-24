<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    documentDetail,
    labelSingleTag,
    tagReviewQueue,
    type DocumentView,
    type TagReviewItem,
    type TagRow,
  } from "./api";
  import { espacenetUrl } from "./espacenet";

  let { tag, onclose }: { tag: TagRow; onclose: () => void } = $props();

  let items = $state<TagReviewItem[]>([]);
  let index = $state(0);
  let doc = $state<DocumentView | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state("");
  let labelled = $state(0);

  const current = $derived(items[index] ?? null);
  const espacenetLink = $derived(doc ? espacenetUrl(doc.pub_key, doc.kind_codes) : null);

  async function loadQueue() {
    loading = true;
    error = "";
    try {
      items = await tagReviewQueue(tag.id);
      index = 0;
      await loadCurrent();
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  async function loadCurrent() {
    doc = null;
    if (!current) return;
    try {
      doc = await documentDetail(current.doc_id);
    } catch (err) {
      error = String(err);
    }
  }

  function move(delta: number) {
    const next = index + delta;
    if (next < 0 || next >= items.length) return;
    index = next;
    loadCurrent();
  }

  async function decide(positive: boolean) {
    if (!current || busy) return;
    busy = true;
    error = "";
    try {
      await labelSingleTag(current.doc_id, tag.id, positive);
      labelled += 1;
      items = items.filter((_, i) => i !== index);
      if (index >= items.length) index = Math.max(0, items.length - 1);
      await loadCurrent();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    if (target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.tagName === "SELECT") return;
    const key = e.key.toLowerCase();
    if (key === "escape") onclose();
    else if (key === "y") decide(true);
    else if (key === "n") decide(false);
    else if (key === "j") move(1);
    else if (key === "k") move(-1);
  }

  $effect(() => {
    loadQueue();
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="tag-review">
  <div class="header">
    <button onclick={onclose} title="Back to tags (Esc)">← Back to tags</button>
    <div class="heading">
      <h1>Review “{tag.name}”</h1>
      <p class="definition">{tag.definition} <span class="muted">(v{tag.version})</span></p>
    </div>
  </div>

  {#if error}<p class="error-banner">{error}</p>{/if}

  {#if loading}
    <p class="muted">Scoring documents…</p>
  {:else if items.length === 0}
    <p class="done">
      Nothing left to review for this tag.
      {#if labelled > 0}{labelled} document{labelled === 1 ? "" : "s"} labelled in this session.{/if}
    </p>
  {:else if current}
    <div class="toolbar">
      <span class="progress">{index + 1} / {items.length}</span>
      <button onclick={() => move(-1)} disabled={index === 0} title="Previous document (K)">← Previous</button>
      <button onclick={() => move(1)} disabled={index >= items.length - 1} title="Next document without deciding (J)">
        Next →
      </button>
      <span class="spacer"></span>
      <button class="no" onclick={() => decide(false)} disabled={busy} title="Does not have this tag (N)">
        No
      </button>
      <button class="yes" onclick={() => decide(true)} disabled={busy} title="Has this tag (Y)">Yes</button>
    </div>

    <section class="document">
      <h2>{current.pub_key}</h2>
      <p class="title">{current.title ?? ""}</p>
      <p class="meta">
        {#if current.score !== null}
          Score {current.score.toFixed(2)}
          <span class="score-bar"><span class="fill" style:width="{current.score * 100}%"></span></span>
        {:else}
          No score yet
        {/if}
        {#if current.previous_state}
          · Previously <strong>{current.previous_state === "pos" ? "yes" : "no"}</strong> under v{current.previous_version}
        {:else}
          · Not labelled for this tag
        {/if}
      </p>
      {#if doc}
        <p class="meta">
          {doc.applicants.join("; ")}
          {#if doc.publication_date}· {doc.publication_date}{/if}
        </p>
        {#if espacenetLink}
          <button class="link" onclick={() => openUrl(espacenetLink)}>Open in Espacenet</button>
        {/if}
        <p class="abstract">{doc.abstract_text}</p>
      {/if}
    </section>

    <div class="shortcuts">
      <kbd>Y</kbd> yes · <kbd>N</kbd> no · <kbd>J</kbd>/<kbd>K</kbd> next/prev · <kbd>Esc</kbd> back
    </div>
  {/if}
</div>

<style>
  .tag-review {
    padding: 1.5rem;
    max-width: 900px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .header {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
  }
  .heading h1 {
    margin: 0;
  }
  .definition {
    margin: 0.25rem 0 0;
  }
  .muted,
  .meta {
    color: var(--muted);
  }
  .meta {
    font-size: 0.85rem;
    margin: 0.25rem 0;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .error-banner {
    color: var(--danger);
  }
  .done {
    color: var(--success);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .progress {
    font-weight: 600;
    margin-right: 0.5rem;
  }
  .spacer {
    flex: 1;
  }
  .yes {
    background: var(--success);
    border-color: var(--success);
    color: var(--bg);
    font-weight: 600;
    min-width: 5rem;
  }
  .no {
    border-color: var(--danger);
    color: var(--danger);
    font-weight: 600;
    min-width: 5rem;
  }
  .title {
    font-weight: 600;
  }
  .score-bar {
    display: inline-block;
    width: 80px;
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
  }
  .score-bar .fill {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .link {
    font-size: 0.8rem;
  }
  .abstract {
    line-height: 1.5;
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
