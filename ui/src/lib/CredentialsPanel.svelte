<script lang="ts">
  import { saveOpsCredentials, testOpsConnection } from "./api";

  let open = $state(false);
  let consumerKey = $state("");
  let consumerSecret = $state("");
  let status = $state<{ kind: "idle" | "busy" | "ok" | "error"; message: string }>({
    kind: "idle",
    message: "",
  });

  async function handleSave() {
    status = { kind: "busy", message: "Saving…" };
    try {
      await saveOpsCredentials(consumerKey, consumerSecret);
      consumerSecret = "";
      status = { kind: "ok", message: "Saved." };
    } catch (err) {
      status = { kind: "error", message: String(err) };
    }
  }

  async function handleTest() {
    status = { kind: "busy", message: "Testing connection…" };
    try {
      await testOpsConnection();
      status = { kind: "ok", message: "Connected." };
    } catch (err) {
      status = { kind: "error", message: String(err) };
    }
  }
</script>

<details class="credentials" bind:open>
  <summary>EPO OPS credentials</summary>
  <div class="fields">
    <label>
      Consumer key
      <input type="text" autocomplete="off" bind:value={consumerKey} />
    </label>
    <label>
      Consumer secret
      <input type="password" autocomplete="off" bind:value={consumerSecret} />
    </label>
    <div class="actions">
      <button onclick={handleSave} disabled={!consumerKey || !consumerSecret}>Save</button>
      <button onclick={handleTest}>Test connection</button>
      {#if status.kind !== "idle"}
        <span class="status {status.kind}">{status.message}</span>
      {/if}
    </div>
  </div>
</details>

<style>
  .credentials {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    margin-bottom: 1rem;
  }
  summary {
    cursor: pointer;
    font-weight: 600;
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    align-items: end;
    margin-top: 0.75rem;
  }
  label {
    display: flex;
    flex-direction: column;
    font-size: 0.85rem;
    gap: 0.25rem;
  }
  input {
    padding: 0.35rem 0.5rem;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .status.ok {
    color: var(--success);
  }
  .status.error {
    color: var(--danger);
  }
  .status.busy {
    color: var(--muted);
  }
</style>
