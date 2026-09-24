<script lang="ts">
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    createBackup,
    dataDirectory,
    exportTagSchema,
    getSettings,
    importTagSchema,
    listTags,
    restoreBackup,
    updateSettings,
    type SettingsView,
    type TagRow,
  } from "./api";
  import CredentialsPanel from "./CredentialsPanel.svelte";

  let settings = $state<SettingsView | null>(null);
  let dataDir = $state("");
  let tags = $state<TagRow[]>([]);
  let status = $state("");
  let error = $state("");
  let busy = $state(false);

  async function refresh() {
    error = "";
    try {
      [settings, dataDir, tags] = await Promise.all([getSettings(), dataDirectory(), listTags()]);
    } catch (err) {
      error = String(err);
    }
  }

  function toggleTagId(list: number[], tagId: number): number[] {
    return list.includes(tagId) ? list.filter((id) => id !== tagId) : [...list, tagId];
  }

  async function handleSave() {
    if (!settings) return;
    busy = true;
    status = "";
    error = "";
    try {
      await updateSettings(settings);
      status = "Saved.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function handleBackup() {
    const path = await saveDialog({ defaultPath: "patent-tagger-backup.zip", filters: [{ name: "Backup", extensions: ["zip"] }] });
    if (!path) return;
    busy = true;
    error = "";
    status = "";
    try {
      await createBackup(path);
      status = `Backed up to ${path}`;
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function handleRestore() {
    const path = await openDialog({ multiple: false, filters: [{ name: "Backup", extensions: ["zip"] }] });
    if (typeof path !== "string") return;
    busy = true;
    error = "";
    status = "";
    try {
      await restoreBackup(path);
      status = "Restored. Restart the app to see the restored data.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function handleExportTagSchema() {
    const path = await saveDialog({ defaultPath: "tags.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    busy = true;
    error = "";
    status = "";
    try {
      await exportTagSchema(path);
      status = `Tags exported to ${path}`;
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function handleImportTagSchema() {
    const path = await openDialog({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    busy = true;
    error = "";
    status = "";
    try {
      const result = await importTagSchema(path);
      const count = result.created.length;
      status = `Imported ${count} new tag${count === 1 ? "" : "s"} (existing tags were left untouched).`;
      if (result.hotkeys_dropped.length > 0) {
        status += ` Hotkey dropped (reserved or already in use): ${result.hotkeys_dropped.join(", ")}.`;
      }
      if (result.parents_dropped.length > 0) {
        status += ` Parent not found, left at top level: ${result.parents_dropped.join(", ")}.`;
      }
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    refresh();
  });
</script>

<div class="settings">
  <h1>Settings</h1>

  {#if error}<p class="error-banner">{error}</p>{/if}
  {#if status}<p class="status">{status}</p>{/if}

  <section>
    <h2>EPO OPS</h2>
    <CredentialsPanel />
  </section>

  {#if settings}
    <section>
      <h2>Automation</h2>
      <label>
        Target precision
        <input type="number" step="0.01" min="0" max="1" bind:value={settings.target_precision} />
      </label>
      <label>
        Target recall
        <input type="number" step="0.01" min="0" max="1" bind:value={settings.target_recall} />
        <span class="note">Used to calibrate each tag's confident-absence threshold.</span>
      </label>
      <label>
        Audit rate
        <input type="number" step="0.01" min="0" max="1" bind:value={settings.audit_rate} />
        <span class="note">Fraction of auto-completed documents routed to review as a full check.</span>
      </label>
      <label class="checkbox">
        <input type="checkbox" bind:checked={settings.full_automation_enabled} />
        Full automation
        <span class="note">Requires at least one tag in automatic mode. Documents where every tag can be confidently decided skip review entirely.</span>
      </label>
    </section>

    <section>
      <h2>Retrieval policies</h2>
      <span class="note">Drawings are heavier and count against a separate OPS quota category - "on demand" is the default.</span>
      <label>
        Full text
        <select bind:value={settings.fulltext_policy}>
          <option value="never">Never</option>
          <option value="on_demand">On demand</option>
          <option value="after_tagging_all">After tagging, for all documents</option>
          <option value="after_tagging_selected_tags">After tagging, for selected tags</option>
        </select>
      </label>
      {#if settings.fulltext_policy === "after_tagging_selected_tags"}
        <div class="tag-checklist">
          {#each tags as tag (tag.id)}
            <label class="checkbox">
              <input
                type="checkbox"
                checked={settings.fulltext_policy_tag_ids.includes(tag.id)}
                onchange={() => settings && (settings.fulltext_policy_tag_ids = toggleTagId(settings.fulltext_policy_tag_ids, tag.id))}
              />
              {tag.name}
            </label>
          {/each}
        </div>
      {/if}
      <label>
        Drawings
        <select bind:value={settings.drawings_policy}>
          <option value="never">Never</option>
          <option value="on_demand">On demand</option>
          <option value="after_tagging_all">After tagging, for all documents</option>
          <option value="after_tagging_selected_tags">After tagging, for selected tags</option>
        </select>
      </label>
      {#if settings.drawings_policy === "after_tagging_selected_tags"}
        <div class="tag-checklist">
          {#each tags as tag (tag.id)}
            <label class="checkbox">
              <input
                type="checkbox"
                checked={settings.drawings_policy_tag_ids.includes(tag.id)}
                onchange={() => settings && (settings.drawings_policy_tag_ids = toggleTagId(settings.drawings_policy_tag_ids, tag.id))}
              />
              {tag.name}
            </label>
          {/each}
        </div>
      {/if}
    </section>

    <button onclick={handleSave} disabled={busy}>Save settings</button>

    <section>
      <h2>Data folder</h2>
      <code>{dataDir}</code>
    </section>

    <section>
      <h2>Backup</h2>
      <div class="actions">
        <button onclick={handleBackup} disabled={busy}>Back up now…</button>
        <button onclick={handleRestore} disabled={busy}>Restore from backup…</button>
      </div>
    </section>

    <section>
      <h2>Tag schema</h2>
      <div class="actions">
        <button onclick={handleExportTagSchema} disabled={busy}>Export tags…</button>
        <button onclick={handleImportTagSchema} disabled={busy}>Import tags…</button>
      </div>
    </section>
  {/if}
</div>

<style>
  .settings {
    padding: 1.5rem;
    max-width: 640px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  h1 {
    margin: 0;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    border-top: 1px solid var(--border);
    padding-top: 1rem;
  }
  section h2 {
    margin: 0;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.9rem;
  }
  label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
  }
  input[type="number"],
  select {
    padding: 0.3rem 0.5rem;
    max-width: 200px;
  }
  .note {
    color: var(--muted);
    font-size: 0.75rem;
  }
  .tag-checklist {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
    padding: 0.4rem 0.6rem;
    background: var(--bg-alt);
    border-radius: 4px;
    font-size: 0.85rem;
  }
  .tag-checklist .checkbox {
    flex-direction: row;
    align-items: center;
    gap: 0.3rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  code {
    font-family: var(--mono);
    font-size: 0.85rem;
    background: var(--bg-alt);
    padding: 0.3rem 0.5rem;
    border-radius: 4px;
    word-break: break-all;
  }
  .error-banner {
    color: var(--danger);
  }
  .status {
    color: var(--success);
  }
</style>
