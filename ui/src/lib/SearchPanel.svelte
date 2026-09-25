<script lang="ts">
  import {
    createSearch,
    deleteSearch,
    fetchSearchBatch,
    listSearches,
    previewSearchQuery,
    restartSearch,
    type BatchReport,
    type SavedSearch,
    type SearchFields,
  } from "./api";

  let { busy, onBatch }: { busy: boolean; onBatch: (report: BatchReport) => Promise<void> } = $props();

  let name = $state("");
  let applicant = $state("");
  let country = $state("");
  let yearFrom = $state<number | null>(null);
  let yearTo = $state<number | null>(null);
  let preview = $state("");
  let searches = $state<SavedSearch[]>([]);
  let working = $state(false);
  let confirmingDelete = $state<number | null>(null);
  let errorMessage = $state("");

  const disabled = $derived(busy || working);

  function fields(): SearchFields {
    return {
      name,
      applicant,
      country: country.trim() || null,
      year_from: yearFrom || null,
      year_to: yearTo || null,
    };
  }

  $effect(() => {
    const current = fields();
    if (!current.applicant.trim()) {
      preview = "";
      return;
    }
    previewSearchQuery(current)
      .then((query) => (preview = query))
      .catch((err) => (preview = String(err)));
  });

  async function refresh() {
    try {
      searches = await listSearches();
    } catch (err) {
      errorMessage = String(err);
    }
  }

  $effect(() => {
    refresh();
  });

  async function run(action: () => Promise<void>) {
    errorMessage = "";
    working = true;
    try {
      await action();
    } catch (err) {
      errorMessage = String(err);
    } finally {
      working = false;
      await refresh();
    }
  }

  function handleSave() {
    return run(async () => {
      await createSearch(fields());
      name = "";
      applicant = "";
      country = "";
      yearFrom = null;
      yearTo = null;
    });
  }

  function handleFetch(search: SavedSearch) {
    return run(async () => {
      const report = await fetchSearchBatch(search.id);
      await refresh();
      await onBatch(report);
    });
  }

  function handleRestart(search: SavedSearch) {
    return run(() => restartSearch(search.id));
  }

  function handleDelete(search: SavedSearch) {
    confirmingDelete = null;
    return run(() => deleteSearch(search.id));
  }

  function progress(search: SavedSearch): string {
    if (search.total_results === null) return "not run yet";
    const capped = search.capped ? " (only the first 2,000 can be read: narrow the search)" : "";
    return `read ${search.results_read} of ${search.total_results} results${capped}, ${search.imported} imported`;
  }
</script>

<div class="search-panel">
  <h2>Search by applicant</h2>

  <div class="form">
    <label>
      Name
      <input bind:value={name} placeholder="e.g. Siemens 2020–2024" disabled={disabled} />
    </label>
    <label class="wide">
      Applicant (separate name variants with ;)
      <input bind:value={applicant} placeholder="Siemens Healthineers; Siemens Healthcare" disabled={disabled} />
    </label>
    <label>
      Country (optional)
      <input bind:value={country} placeholder="EP" maxlength="2" class="short" disabled={disabled} />
    </label>
    <label>
      Published from (optional)
      <input type="number" bind:value={yearFrom} placeholder="2020" class="short" disabled={disabled} />
    </label>
    <label>
      to (optional)
      <input type="number" bind:value={yearTo} placeholder="2024" class="short" disabled={disabled} />
    </label>
  </div>
  {#if preview}
    <p class="query">Query: <code>{preview}</code></p>
  {/if}
  <div class="toolbar">
    <button onclick={handleSave} disabled={disabled || !name.trim() || !applicant.trim()}>Save search</button>
  </div>

  {#if errorMessage}
    <p class="error-banner">{errorMessage}</p>
  {/if}

  {#if searches.length > 0}
    <ul class="searches">
      {#each searches as search (search.id)}
        <li>
          <div class="info">
            <span class="name">{search.name}</span>
            <code class="detail">{search.query}</code>
            <span class="detail">{progress(search)}</span>
          </div>
          <div class="actions">
            {#if search.exhausted}
              <button onclick={() => handleRestart(search)} disabled={disabled}>Start over</button>
            {:else}
              <button onclick={() => handleFetch(search)} disabled={disabled}>Fetch next 100</button>
            {/if}
            {#if confirmingDelete === search.id}
              <button class="danger" onclick={() => handleDelete(search)} disabled={disabled}>Confirm delete</button>
              <button onclick={() => (confirmingDelete = null)} disabled={disabled}>Cancel</button>
            {:else}
              <button onclick={() => (confirmingDelete = search.id)} disabled={disabled}>Delete</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .search-panel {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    border-top: 1px solid var(--border);
    padding-top: 1rem;
  }
  h2 {
    margin: 0;
  }
  .form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.9rem;
  }
  label.wide {
    flex: 1 1 100%;
  }
  input {
    padding: 0.3rem 0.5rem;
  }
  input.short {
    max-width: 8rem;
  }
  .query {
    margin: 0;
    font-size: 0.85rem;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
  .toolbar {
    display: flex;
    gap: 0.5rem;
  }
  .error-banner {
    color: var(--danger);
  }
  .searches {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .searches li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.5rem 0.6rem;
    background: var(--bg-alt);
    border-radius: 4px;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }
  .name {
    font-weight: 600;
  }
  .detail {
    color: var(--muted);
    font-size: 0.8rem;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    gap: 0.4rem;
  }
</style>
