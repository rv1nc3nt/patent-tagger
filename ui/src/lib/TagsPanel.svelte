<script lang="ts">
  import { archiveTag, createTag, listTags, type TagRow } from "./api";

  let { onchange }: { onchange?: () => void } = $props();

  let open = $state(false);
  let tags = $state<TagRow[]>([]);
  let name = $state("");
  let definition = $state("");
  let color = $state("");
  let hotkey = $state("");
  let error = $state("");
  let busy = $state(false);

  async function refresh() {
    tags = await listTags();
    onchange?.();
  }

  $effect(() => {
    if (open) refresh();
  });

  async function handleCreate() {
    error = "";
    busy = true;
    try {
      await createTag(name, definition, color || null, hotkey || null);
      name = "";
      definition = "";
      color = "";
      hotkey = "";
      await refresh();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function handleArchive(tagId: number) {
    await archiveTag(tagId);
    await refresh();
  }
</script>

<details class="tags-panel" bind:open>
  <summary>Tags ({tags.length})</summary>
  <div class="body">
    <ul class="tag-list">
      {#each tags as tag (tag.id)}
        <li>
          <span class="swatch" style:background={tag.color ?? "var(--border)"}></span>
          <span class="name">{tag.name}</span>
          {#if tag.hotkey}<kbd>{tag.hotkey}</kbd>{/if}
          <button class="archive" onclick={() => handleArchive(tag.id)}>Archive</button>
        </li>
      {/each}
    </ul>

    <div class="new-tag">
      <input placeholder="Name" bind:value={name} disabled={busy} />
      <input placeholder="Definition" bind:value={definition} disabled={busy} />
      <input placeholder="Colour (#hex)" bind:value={color} disabled={busy} />
      <input placeholder="Hotkey" maxlength="1" bind:value={hotkey} disabled={busy} />
      <button onclick={handleCreate} disabled={busy || !name.trim() || !definition.trim()}>
        Add tag
      </button>
    </div>
    {#if error}<p class="error">{error}</p>{/if}
  </div>
</details>

<style>
  .tags-panel {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
  }
  summary {
    cursor: pointer;
    font-weight: 600;
  }
  .body {
    margin-top: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .tag-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .tag-list li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .swatch {
    width: 0.75rem;
    height: 0.75rem;
    border-radius: 50%;
    display: inline-block;
    border: 1px solid var(--border);
  }
  .name {
    flex: 1;
  }
  kbd {
    font-family: var(--mono);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 0.3rem;
    font-size: 0.75rem;
  }
  .archive {
    font-size: 0.75rem;
  }
  .new-tag {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .new-tag input {
    padding: 0.3rem 0.5rem;
    flex: 1 1 120px;
  }
  .error {
    color: var(--danger);
    margin: 0;
  }
</style>
