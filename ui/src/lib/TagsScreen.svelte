<script lang="ts">
  import {
    archiveTag,
    createTag,
    disableAutomaticMode,
    discardStaleLabels,
    enableAutomaticMode,
    listArchivedTags,
    tagOverview,
    unarchiveTag,
    updateTag,
    type TagFields,
    type TagOverview,
    type TagRow,
  } from "./api";
  import TagReview from "./TagReview.svelte";
  import { treeOrder } from "./tagTree";

  let overview = $state<TagOverview[]>([]);
  let archived = $state<TagRow[]>([]);
  let selectedId = $state<number | "new" | null>(null);
  let reviewing = $state<TagRow | null>(null);
  let error = $state("");
  let notice = $state("");
  let busy = $state(false);
  let confirmingDiscard = $state(false);

  let name = $state("");
  let definition = $state("");
  let parentId = $state<number | null>(null);
  let color = $state<string | null>(null);
  let hotkey = $state("");
  let bumpOverride = $state<boolean | null>(null);

  const tree = $derived(
    treeOrder(
      overview,
      (o) => o.tag.id,
      (o) => o.tag.parent_id,
      (o) => o.tag.name,
    ),
  );
  const selected = $derived(
    typeof selectedId === "number" ? (overview.find((o) => o.tag.id === selectedId) ?? null) : null,
  );
  const definitionChanged = $derived(selected !== null && definition.trim() !== selected.tag.definition);
  // A material change bumps the version (SPEC 7.1). Defaults to "yes" when the
  // definition text changed, until the user decides otherwise.
  const bumpVersion = $derived(bumpOverride ?? definitionChanged);

  /// The tag itself and its descendants cannot be its parent.
  const parentOptions = $derived.by(() => {
    if (selected === null) return tree;
    const excluded = new Set([selected.tag.id]);
    let grew = true;
    while (grew) {
      grew = false;
      for (const o of overview) {
        if (o.tag.parent_id !== null && excluded.has(o.tag.parent_id) && !excluded.has(o.tag.id)) {
          excluded.add(o.tag.id);
          grew = true;
        }
      }
    }
    return tree.filter(({ item }) => !excluded.has(item.tag.id));
  });

  async function refresh() {
    try {
      [overview, archived] = await Promise.all([tagOverview(), listArchivedTags()]);
      if (typeof selectedId === "number" && !overview.some((o) => o.tag.id === selectedId)) {
        selectedId = null;
      }
    } catch (err) {
      error = String(err);
    }
  }

  function loadForm(tag: TagRow | null) {
    name = tag?.name ?? "";
    definition = tag?.definition ?? "";
    parentId = tag?.parent_id ?? null;
    color = tag?.color ?? null;
    hotkey = tag?.hotkey ?? "";
    bumpOverride = null;
    confirmingDiscard = false;
  }

  function select(tag: TagRow) {
    selectedId = tag.id;
    loadForm(tag);
    error = "";
    notice = "";
  }

  function startNew() {
    selectedId = "new";
    loadForm(null);
    error = "";
    notice = "";
  }

  async function run(action: () => Promise<void>) {
    busy = true;
    error = "";
    notice = "";
    try {
      await action();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function fields(): TagFields {
    return { name, definition, parentId, color, hotkey: hotkey || null };
  }

  function handleSave() {
    run(async () => {
      if (selectedId === "new") {
        const tag = await createTag(fields());
        await refresh();
        select(tag);
        notice = `Created "${tag.name}".`;
      } else if (selected) {
        const tag = await updateTag(selected.tag.id, fields(), bumpVersion);
        await refresh();
        select(tag);
        notice =
          tag.version > selected.tag.version
            ? `Saved as version ${tag.version}. Older labels are now open to re-review.`
            : "Saved.";
      }
    });
  }

  function handleArchive(tag: TagRow) {
    run(async () => {
      await archiveTag(tag.id);
      selectedId = null;
      await refresh();
      notice = `Archived "${tag.name}". It can be restored from the archived list.`;
    });
  }

  function handleRestore(tag: TagRow) {
    run(async () => {
      const restored = await unarchiveTag(tag.id);
      await refresh();
      select(restored.tag);
      notice = restored.hotkey_cleared
        ? `Restored "${tag.name}". Its hotkey "${restored.hotkey_cleared}" is now used by another tag and was removed.`
        : `Restored "${tag.name}".`;
    });
  }

  function handleToggleAuto(o: TagOverview) {
    run(async () => {
      if (o.tag.auto_enabled) await disableAutomaticMode(o.tag.id);
      else await enableAutomaticMode(o.tag.id);
      await refresh();
    });
  }

  function handleDiscard(o: TagOverview) {
    run(async () => {
      const n = await discardStaleLabels(o.tag.id);
      confirmingDiscard = false;
      await refresh();
      notice = `Discarded ${n} stale label${n === 1 ? "" : "s"}. Those documents are back in the review queue.`;
    });
  }

  async function closeReview() {
    reviewing = null;
    await refresh();
  }

  $effect(() => {
    refresh();
  });
</script>

{#if reviewing}
  <TagReview tag={reviewing} onclose={closeReview} />
{:else}
  <div class="tags-screen">
    <div class="header">
      <h1>Tags</h1>
      <button class="primary" onclick={startNew}>New tag</button>
    </div>

    {#if error}<p class="error-banner">{error}</p>{/if}
    {#if notice}<p class="notice">{notice}</p>{/if}

    <div class="panes">
      <section class="list-pane">
        {#if overview.length === 0}
          <p class="muted">No tags yet.</p>
        {/if}
        <ul class="tag-list">
          {#each tree as { item: o, depth } (o.tag.id)}
            {@const toReview = o.stats.unlabelled + o.stats.stale}
            <li>
              <button
                class="tag-row"
                class:current={selectedId === o.tag.id}
                style:padding-left="{0.5 + depth * 1.1}rem"
                onclick={() => select(o.tag)}
              >
                <span class="swatch" style:background={o.tag.color ?? "transparent"}></span>
                <span class="name">{o.tag.name}</span>
                {#if o.tag.auto_enabled}<span class="badge">auto</span>{/if}
                {#if toReview > 0}<span class="count" title="Documents to review for this tag">{toReview}</span>{/if}
                {#if o.tag.hotkey}<kbd>{o.tag.hotkey}</kbd>{/if}
              </button>
            </li>
          {/each}
        </ul>

        {#if archived.length > 0}
          <details class="archived">
            <summary>Archived ({archived.length})</summary>
            <ul>
              {#each archived as tag (tag.id)}
                <li>
                  <span class="name">{tag.name}</span>
                  <button onclick={() => handleRestore(tag)} disabled={busy}>Restore</button>
                </li>
              {/each}
            </ul>
          </details>
        {/if}
      </section>

      <section class="detail-pane">
        {#if selectedId === null}
          <p class="muted">Select a tag to edit it, or create a new one.</p>
        {:else}
          <h2>{selectedId === "new" ? "New tag" : `Edit "${selected?.tag.name}"`}</h2>
          <form
            class="tag-form"
            onsubmit={(e) => {
              e.preventDefault();
              handleSave();
            }}
          >
            <label>
              Name
              <input bind:value={name} disabled={busy} required />
            </label>
            <label>
              Definition
              <textarea bind:value={definition} rows="4" disabled={busy} required></textarea>
            </label>
            <label>
              Parent
              <select
                value={parentId ?? ""}
                onchange={(e) => (parentId = e.currentTarget.value === "" ? null : Number(e.currentTarget.value))}
                disabled={busy}
              >
                <option value="">None (top level)</option>
                {#each parentOptions as { item: o, depth } (o.tag.id)}
                  <option value={o.tag.id}>{"  ".repeat(depth)}{o.tag.name}</option>
                {/each}
              </select>
            </label>
            <div class="row">
              <label>
                Colour
                <span class="colour-field">
                  <input
                    type="color"
                    value={color ?? "#888888"}
                    oninput={(e) => (color = e.currentTarget.value)}
                    disabled={busy}
                  />
                  {#if color}
                    <button type="button" onclick={() => (color = null)} disabled={busy}>No colour</button>
                  {:else}
                    <span class="muted">none</span>
                  {/if}
                </span>
              </label>
              <label>
                Hotkey
                <input class="hotkey" maxlength="1" bind:value={hotkey} disabled={busy} />
              </label>
            </div>
            <p class="hint">J, K, S and / are reserved by the Review screen.</p>

            {#if selected}
              <label class="checkbox">
                <input
                  type="checkbox"
                  checked={bumpVersion}
                  onchange={(e) => (bumpOverride = e.currentTarget.checked)}
                  disabled={busy}
                />
                Material change: bump to version {selected.tag.version + 1}
              </label>
              {#if bumpVersion}
                <p class="hint">
                  Existing labels stay usable for training and become open to re-review against the new definition.
                </p>
              {/if}
            {/if}

            <div class="actions">
              <button class="primary" type="submit" disabled={busy || !name.trim() || !definition.trim()}>
                {selectedId === "new" ? "Create tag" : "Save"}
              </button>
              <button
                type="button"
                onclick={() => (selected ? loadForm(selected.tag) : (selectedId = null))}
                disabled={busy}
              >
                Cancel
              </button>
              {#if selected}
                <span class="spacer"></span>
                <button type="button" onclick={() => handleArchive(selected.tag)} disabled={busy}>Archive</button>
              {/if}
            </div>
          </form>

          {#if selected}
            {@const s = selected.stats}
            {@const e = selected.eligibility}
            <h3>Labels</h3>
            <table class="stats">
              <tbody>
                <tr><th>Human</th><td>{s.human_pos} positive · {s.human_neg} negative</td></tr>
                <tr><th>Automatic</th><td>{s.auto_pos} positive · {s.auto_neg} negative</td></tr>
                <tr><th>Unknown</th><td>{s.unlabelled} reviewed document{s.unlabelled === 1 ? "" : "s"} without a label</td></tr>
                <tr>
                  <th>Stale</th>
                  <td>
                    {s.stale} human label{s.stale === 1 ? "" : "s"} from an older version (current: v{selected.tag.version})
                  </td>
                </tr>
              </tbody>
            </table>
            <div class="actions">
              <button
                onclick={() => (reviewing = selected.tag)}
                disabled={busy || s.unlabelled + s.stale === 0}
              >
                Review against existing documents ({s.unlabelled + s.stale})
              </button>
              {#if s.stale > 0}
                {#if confirmingDiscard}
                  <button class="danger" onclick={() => handleDiscard(selected)} disabled={busy}>
                    Confirm: discard {s.stale} stale label{s.stale === 1 ? "" : "s"}
                  </button>
                  <button onclick={() => (confirmingDiscard = false)} disabled={busy}>Cancel</button>
                {:else}
                  <button
                    onclick={() => (confirmingDiscard = true)}
                    disabled={busy}
                    title="Stop using the older labels for training. Their documents become unknown for this tag."
                  >
                    Discard stale labels…
                  </button>
                {/if}
              {/if}
            </div>

            <h3>Automatic mode</h3>
            <p>
              {#if selected.tag.auto_enabled}
                Enabled.
              {:else if e.eligible}
                Eligible.
              {:else}
                Not eligible yet ({e.n_pos} positives, {e.n_evaluated} evaluated predictions).
              {/if}
            </p>
            <div class="actions">
              <button
                onclick={() => handleToggleAuto(selected)}
                disabled={busy || (!selected.tag.auto_enabled && !e.eligible)}
              >
                {selected.tag.auto_enabled ? "Disable automatic mode" : "Enable automatic mode"}
              </button>
            </div>
          {/if}
        {/if}
      </section>
    </div>
  </div>
{/if}

<style>
  .tags-screen {
    padding: 1.5rem;
    max-width: 1100px;
    margin: 0 auto;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }
  .header h1 {
    margin: 0;
  }
  .error-banner {
    color: var(--danger);
  }
  .notice {
    color: var(--success);
  }
  .muted {
    color: var(--muted);
  }
  .panes {
    display: grid;
    grid-template-columns: 300px 1fr;
    gap: 1.5rem;
    align-items: start;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .tag-list {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .tag-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    text-align: left;
    border: none;
    background: transparent;
    padding: 0.3rem 0.5rem;
  }
  .tag-row.current {
    background: var(--bg-alt);
    font-weight: 600;
  }
  .swatch {
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 50%;
    border: 1px solid var(--border);
    flex: none;
  }
  .name {
    flex: 1;
  }
  .badge {
    font-size: 0.65rem;
    text-transform: uppercase;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 3px;
    padding: 0 0.25rem;
  }
  .count {
    font-size: 0.7rem;
    background: var(--series-recall);
    color: #000;
    border-radius: 999px;
    padding: 0 0.4rem;
  }
  kbd {
    font-family: var(--mono);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 0.3rem;
    font-size: 0.75rem;
  }
  .archived {
    margin-top: 1rem;
    border-top: 1px solid var(--border);
    padding-top: 0.5rem;
  }
  .archived summary {
    cursor: pointer;
    color: var(--muted);
  }
  .archived li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0;
  }
  .archived button {
    font-size: 0.8rem;
    padding: 0.2rem 0.5rem;
  }
  .detail-pane h3 {
    margin-top: 1.5rem;
    margin-bottom: 0.5rem;
  }
  .tag-form {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin-top: 0.75rem;
  }
  .tag-form label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.85rem;
    color: var(--muted);
  }
  .tag-form input,
  .tag-form textarea,
  .tag-form select {
    padding: 0.35rem 0.5rem;
    color: var(--text);
    font: inherit;
  }
  .tag-form label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
    color: var(--text);
  }
  .row {
    display: flex;
    gap: 1rem;
  }
  .colour-field {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .colour-field input {
    width: 3rem;
    height: 2rem;
    padding: 0.1rem;
  }
  .hotkey {
    width: 3rem;
    text-align: center;
  }
  .hint {
    font-size: 0.8rem;
    color: var(--muted);
    margin: 0;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .spacer {
    flex: 1;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
    font-weight: 600;
  }
  .danger {
    border-color: var(--danger);
    color: var(--danger);
  }
  .stats {
    border-collapse: collapse;
    margin-bottom: 0.75rem;
  }
  .stats th,
  .stats td {
    text-align: left;
    padding: 0.25rem 1rem 0.25rem 0;
    font-size: 0.9rem;
  }
  .stats th {
    color: var(--muted);
    font-weight: 600;
  }
</style>
