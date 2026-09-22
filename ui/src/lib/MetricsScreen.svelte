<script lang="ts">
  import { retrainNow, tagMetrics, type TagMetrics } from "./api";
  import PrCurveChart from "./PrCurveChart.svelte";

  let metrics = $state<TagMetrics[]>([]);
  let error = $state("");
  let retraining = $state(false);
  let retrainMessage = $state("");

  async function refresh() {
    error = "";
    try {
      metrics = await tagMetrics();
    } catch (err) {
      error = String(err);
    }
  }

  async function handleRetrain() {
    retraining = true;
    retrainMessage = "";
    try {
      const count = await retrainNow();
      retrainMessage = `Retrained ${count} tag${count === 1 ? "" : "s"}.`;
      await refresh();
    } catch (err) {
      error = String(err);
    } finally {
      retraining = false;
    }
  }

  function fmt(value: number | null): string {
    return value === null ? "—" : value.toFixed(2);
  }

  $effect(() => {
    refresh();
  });
</script>

<div class="metrics">
  <div class="header">
    <h1>Metrics</h1>
    <button onclick={handleRetrain} disabled={retraining}>Retrain now</button>
    {#if retrainMessage}<span class="note">{retrainMessage}</span>{/if}
  </div>

  {#if error}<p class="error-banner">{error}</p>{/if}

  {#if metrics.length === 0}
    <p class="empty">No tags yet.</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>Tag</th>
          <th>Support</th>
          <th>Precision</th>
          <th>Recall</th>
          <th>PR curve</th>
        </tr>
      </thead>
      <tbody>
        {#each metrics as row (row.tag_id)}
          <tr>
            <td>{row.name}</td>
            <td>{row.support_pos} / {row.support_total}</td>
            <td>{fmt(row.precision)}</td>
            <td>{fmt(row.recall)}</td>
            <td><PrCurveChart points={row.pr_curve} /></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .metrics {
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
  .note {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .error-banner {
    color: var(--danger);
  }
  .empty {
    color: var(--muted);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  th {
    color: var(--muted);
    font-weight: 600;
    font-size: 0.8rem;
  }
</style>
