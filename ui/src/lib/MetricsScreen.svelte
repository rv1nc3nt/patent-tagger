<script lang="ts">
  import {
    disableAutomaticMode,
    enableAutomaticMode,
    listTags,
    retrainNow,
    tagEligibility,
    tagMetrics,
    type Eligibility,
    type TagMetrics,
    type TagRow,
  } from "./api";
  import PrCurveChart from "./PrCurveChart.svelte";

  interface Row {
    tag: TagRow;
    metrics: TagMetrics;
    eligibility: Eligibility;
  }

  let rows = $state<Row[]>([]);
  let error = $state("");
  let retraining = $state(false);
  let retrainMessage = $state("");
  let togglingTagId = $state<number | null>(null);

  async function refresh() {
    error = "";
    try {
      const [tags, metrics] = await Promise.all([listTags(), tagMetrics()]);
      const eligibilities = await Promise.all(tags.map((t) => tagEligibility(t.id)));
      rows = tags.map((tag, i) => ({
        tag,
        metrics: metrics.find((m) => m.tag_id === tag.id)!,
        eligibility: eligibilities[i],
      }));
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

  async function handleToggleAuto(row: Row) {
    togglingTagId = row.tag.id;
    error = "";
    try {
      if (row.tag.auto_enabled) {
        await disableAutomaticMode(row.tag.id);
      } else {
        await enableAutomaticMode(row.tag.id);
      }
      await refresh();
    } catch (err) {
      error = String(err);
    } finally {
      togglingTagId = null;
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

  {#if rows.length === 0}
    <p class="empty">No tags yet.</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>Tag</th>
          <th>Support</th>
          <th>Precision</th>
          <th>Recall</th>
          <th>Threshold</th>
          <th>Automatic mode</th>
          <th>PR curve</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as row (row.tag.id)}
          <tr>
            <td>{row.tag.name}</td>
            <td>{row.metrics.support_pos} / {row.metrics.support_total}</td>
            <td>{fmt(row.metrics.precision)}</td>
            <td>{fmt(row.metrics.recall)}</td>
            <td>{row.tag.threshold === null ? "not calibrated" : row.tag.threshold.toFixed(2)}</td>
            <td>
              <button
                onclick={() => handleToggleAuto(row)}
                disabled={togglingTagId === row.tag.id || (!row.tag.auto_enabled && !row.eligibility.eligible)}
                title={row.eligibility.eligible
                  ? ""
                  : `not yet eligible: ${row.eligibility.n_pos}/30 positives, ${row.eligibility.n_evaluated}/150 evaluated`}
              >
                {row.tag.auto_enabled ? "Disable" : "Enable"}
              </button>
              {#if !row.tag.auto_enabled && !row.eligibility.eligible}
                <span class="note">not eligible yet</span>
              {/if}
            </td>
            <td><PrCurveChart points={row.metrics.pr_curve} /></td>
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
    font-size: 0.75rem;
    display: block;
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
