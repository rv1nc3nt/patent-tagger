<script lang="ts">
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    bulkRetrieve,
    exportCsv,
    exportDocuments,
    exportJson,
    exportTagList,
    librarySearch,
    listTags,
    similarDocuments,
    type LibraryRow,
    type SimilarDocument,
    type TagRow,
  } from "./api";
  import DrawingPage from "./DrawingPage.svelte";

  let { onView }: { onView?: (docId: number) => void } = $props();

  let tags = $state<TagRow[]>([]);
  let rows = $state<LibraryRow[]>([]);
  let query = $state("");
  let includeTagIds = $state<Set<number>>(new Set());
  let excludeTagIds = $state<Set<number>>(new Set());
  let labelSource = $state<string>("");
  let reviewState = $state<string>("");
  let fulltextAvailability = $state<string>("");
  let drawingsAvailability = $state<string>("");
  let selected = $state<Set<number>>(new Set());
  let similarFor = $state<number | null>(null);
  let similarResults = $state<SimilarDocument[]>([]);
  let status = $state("");
  let error = $state("");
  let retrieving = $state(false);

  async function loadTags() {
    tags = await listTags();
  }

  async function runSearch() {
    error = "";
    try {
      rows = await librarySearch({
        query: query.trim() || null,
        include_tag_ids: Array.from(includeTagIds),
        exclude_tag_ids: Array.from(excludeTagIds),
        label_source: labelSource || null,
        review_state: reviewState || null,
        fulltext_availability: fulltextAvailability || null,
        drawings_availability: drawingsAvailability || null,
      });
      selected = new Set();
    } catch (err) {
      error = String(err);
    }
  }

  function toggleSet(set: Set<number>, id: number): Set<number> {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  }

  function toggleSelected(id: number) {
    selected = toggleSet(selected, id);
  }

  async function handleShowSimilar(id: number) {
    error = "";
    try {
      similarFor = id;
      similarResults = await similarDocuments(id, 10);
    } catch (err) {
      error = String(err);
    }
  }

  async function handleExportCsv() {
    const path = await saveDialog({ defaultPath: "library.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return;
    try {
      await exportCsv(path);
      status = `Exported to ${path}`;
    } catch (err) {
      error = String(err);
    }
  }

  async function handleExportJson() {
    const path = await saveDialog({ defaultPath: "library.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    try {
      await exportJson(path);
      status = `Exported to ${path}`;
    } catch (err) {
      error = String(err);
    }
  }

  async function handleExportTagList(tagId: number) {
    const tag = tags.find((t) => t.id === tagId);
    const path = await saveDialog({ defaultPath: `${tag?.name ?? "tag"}.txt`, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (!path) return;
    try {
      await exportTagList(tagId, path);
      status = `Exported to ${path}`;
    } catch (err) {
      error = String(err);
    }
  }

  async function handleExportSelected() {
    if (selected.size === 0) return;
    const dir = await openDialog({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    try {
      const count = await exportDocuments(Array.from(selected), dir);
      status = `Exported ${count} document${count === 1 ? "" : "s"} to ${dir}`;
    } catch (err) {
      error = String(err);
    }
  }

  async function handleBulkRetrieve(kind: "fulltext" | "drawings") {
    if (selected.size === 0) return;
    retrieving = true;
    error = "";
    status = "";
    try {
      await bulkRetrieve(Array.from(selected), kind);
      status = `Retrieved ${kind === "fulltext" ? "full text" : "drawings"} for ${selected.size} document(s).`;
      await runSearch();
    } catch (err) {
      error = String(err);
    } finally {
      retrieving = false;
    }
  }

  $effect(() => {
    loadTags();
    runSearch();
  });
</script>

<div class="library">
  <h1>Library</h1>

  {#if error}<p class="error-banner">{error}</p>{/if}
  {#if status}<p class="status">{status}</p>{/if}

  <div class="filters">
    <input placeholder="Search title/abstract…" bind:value={query} onkeydown={(e) => e.key === "Enter" && runSearch()} />
    <select bind:value={labelSource}>
      <option value="">Any label source</option>
      <option value="human">Human</option>
      <option value="auto">Automatic</option>
    </select>
    <select bind:value={reviewState}>
      <option value="">Any review state</option>
      <option value="queued">Queued</option>
      <option value="validated">Validated</option>
      <option value="skipped">Skipped</option>
    </select>
    <select bind:value={fulltextAvailability}>
      <option value="">Any full-text availability</option>
      <option value="available">Full text available</option>
      <option value="unavailable">Full text unavailable</option>
    </select>
    <select bind:value={drawingsAvailability}>
      <option value="">Any drawings availability</option>
      <option value="available">Drawings available</option>
      <option value="unavailable">Drawings unavailable</option>
    </select>
    <button onclick={runSearch}>Search</button>
  </div>

  <div class="tag-filters">
    {#each tags as tag (tag.id)}
      <span class="tag-chip">
        {tag.name}
        <button
          class:active={includeTagIds.has(tag.id)}
          title="Include"
          onclick={() => {
            includeTagIds = toggleSet(includeTagIds, tag.id);
            runSearch();
          }}
        >
          +
        </button>
        <button
          class:active={excludeTagIds.has(tag.id)}
          title="Exclude"
          onclick={() => {
            excludeTagIds = toggleSet(excludeTagIds, tag.id);
            runSearch();
          }}
        >
          −
        </button>
        <button class="link" onclick={() => handleExportTagList(tag.id)}>list</button>
      </span>
    {/each}
  </div>

  <div class="bulk-actions">
    <button onclick={handleExportCsv}>Export CSV</button>
    <button onclick={handleExportJson}>Export JSON</button>
    <button onclick={handleExportSelected} disabled={selected.size === 0}>
      Export selected ({selected.size})…
    </button>
    <button onclick={() => handleBulkRetrieve("fulltext")} disabled={selected.size === 0 || retrieving}>
      Retrieve full text for selected
    </button>
    <button onclick={() => handleBulkRetrieve("drawings")} disabled={selected.size === 0 || retrieving}>
      Retrieve drawings for selected
    </button>
  </div>

  <table>
    <thead>
      <tr>
        <th></th>
        <th></th>
        <th>Publication</th>
        <th>Title</th>
        <th>Review state</th>
        <th>Tags</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.id)}
        <tr>
          <td><input type="checkbox" checked={selected.has(row.id)} onchange={() => toggleSelected(row.id)} /></td>
          <td>
            {#if row.has_drawings}
              <DrawingPage docId={row.id} page={0} size={40} />
            {/if}
          </td>
          <td>{row.pub_key}</td>
          <td>{row.title ?? "—"}</td>
          <td>{row.review_state}</td>
          <td>{row.tags.join(", ")}</td>
          <td>
            {#if onView}<button class="link" onclick={() => onView(row.id)}>view</button>{/if}
            <button class="link" onclick={() => handleShowSimilar(row.id)}>similar</button>
          </td>
        </tr>
        {#if similarFor === row.id}
          <tr class="similar-row">
            <td colspan="7">
              {#if similarResults.length === 0}
                <span class="note">No similar documents yet.</span>
              {:else}
                <ul>
                  {#each similarResults as sim (sim.id)}
                    <li>{sim.pub_key} — {sim.title ?? "…"} (similarity {sim.similarity.toFixed(2)})</li>
                  {/each}
                </ul>
              {/if}
            </td>
          </tr>
        {/if}
      {/each}
    </tbody>
  </table>
  {#if rows.length === 0}<p class="note">No documents match.</p>{/if}
</div>

<style>
  .library {
    padding: 1.5rem;
    max-width: 1100px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  h1 {
    margin: 0;
  }
  .error-banner {
    color: var(--danger);
  }
  .status {
    color: var(--success);
  }
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .filters {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .filters input {
    flex: 1 1 240px;
    padding: 0.35rem 0.5rem;
  }
  .filters select {
    padding: 0.35rem 0.5rem;
  }
  .tag-filters {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.15rem;
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 0.1rem 0.4rem;
    font-size: 0.8rem;
  }
  .tag-chip button {
    padding: 0 0.35rem;
    font-size: 0.75rem;
  }
  .tag-chip button.active {
    background: var(--accent);
    color: white;
  }
  .bulk-actions {
    display: flex;
    gap: 0.5rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0.4rem 0.6rem;
    border-bottom: 1px solid var(--border);
    font-size: 0.85rem;
  }
  .similar-row td {
    background: var(--bg-alt);
  }
  .similar-row ul {
    margin: 0;
    padding-left: 1.25rem;
  }
  .link {
    font-size: 0.75rem;
  }
</style>
