<script lang="ts">
  import {
    disableAutomaticMode,
    enableAutomaticMode,
    fullAutomationSummary,
    listTags,
    retrainNow,
    tagEligibility,
    tagMetrics,
    type Eligibility,
    type FullAutomationSummary,
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
  let summary = $state<FullAutomationSummary | null>(null);
  let error = $state("");
  let retraining = $state(false);
  let retrainMessage = $state("");
  let togglingTagId = $state<number | null>(null);

  async function refresh() {
    error = "";
    try {
      const [tags, metrics, automationSummary] = await Promise.all([
        listTags(),
        tagMetrics(),
        fullAutomationSummary(),
      ]);
      const eligibilities = await Promise.all(tags.map((t) => tagEligibility(t.id)));
      rows = tags.map((tag, i) => ({
        tag,
        metrics: metrics.find((m) => m.tag_id === tag.id)!,
        eligibility: eligibilities[i],
      }));
      summary = automationSummary;
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

  {#if summary}
    <div class="readiness">
      <div class="readiness-stat">
        <span class="value">{summary.total > 0 ? Math.round((100 * summary.would_auto_complete) / summary.total) : 0}%</span>
        <span class="label">of the last {summary.total} documents would auto-complete with current settings</span>
      </div>
      <div class="readiness-counts">
        <span><strong>{summary.auto_completed}</strong> auto-completed</span>
        <span><strong>{summary.audited_or_complete}</strong> audited / fully decided</span>
        <span><strong>{summary.focused_review}</strong> in focused review</span>
      </div>
    </div>
  {/if}

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
          <th>Neg. threshold</th>
          <th>Audit</th>
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
            <td>{row.tag.neg_threshold === null ? "not calibrated" : row.tag.neg_threshold.toFixed(2)}</td>
            <td class="audit-cell">
              <span title="Audited precision (per-tag automatic mode safeguard)">
                P {fmt(row.metrics.audited_precision)} (n={row.metrics.audited_n})
              </span>
              <span title="Full-automation audit precision/recall (n from a shared window)">
                Full: P {fmt(row.metrics.full_automation_precision)} / R {fmt(row.metrics.full_automation_recall)} (n={row.metrics.full_automation_n})
              </span>
            </td>
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
  .readiness {
    display: flex;
    align-items: center;
    gap: 2rem;
    padding: 0.75rem 1rem;
    margin-bottom: 1rem;
    background: var(--bg-alt);
    border-radius: 6px;
  }
  .readiness-stat {
    display: flex;
    flex-direction: column;
  }
  .readiness-stat .value {
    font-size: 1.6rem;
    font-weight: 700;
  }
  .readiness-stat .label {
    font-size: 0.75rem;
    color: var(--muted);
    max-width: 220px;
  }
  .readiness-counts {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.85rem;
  }
  .audit-cell {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    white-space: nowrap;
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
