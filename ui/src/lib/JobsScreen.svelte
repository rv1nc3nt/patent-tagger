<script lang="ts">
  import {
    cancelPendingJobs,
    retryFailedJobs,
    retryJob,
    type JobKind,
    type JobOverview,
  } from "./api";

  let { overview, refresh }: { overview: JobOverview | null; refresh: () => Promise<void> } = $props();

  let working = $state(false);
  let confirmingCancel = $state<JobKind | null>(null);
  let status = $state("");
  let errorMessage = $state("");

  const kindLabels: Record<JobKind, string> = {
    import_document: "Import",
    fulltext_retrieval: "Full text",
    drawings_retrieval: "Drawings",
  };
  const cancellable: JobKind[] = ["fulltext_retrieval", "drawings_retrieval"];

  async function run(action: () => Promise<string>) {
    errorMessage = "";
    status = "";
    working = true;
    try {
      status = await action();
    } catch (err) {
      errorMessage = String(err);
    } finally {
      working = false;
      await refresh();
    }
  }

  function handleRetry(jobId: number) {
    return run(async () => {
      await retryJob(jobId);
      return "Job queued again.";
    });
  }

  function handleRetryAll(kind: JobKind) {
    return run(async () => {
      const n = await retryFailedJobs(kind);
      return `${n} ${kindLabels[kind].toLowerCase()} job${n === 1 ? "" : "s"} queued again.`;
    });
  }

  function handleCancel(kind: JobKind) {
    confirmingCancel = null;
    return run(async () => {
      const n = await cancelPendingJobs(kind);
      return `${n} pending ${kindLabels[kind].toLowerCase()} job${n === 1 ? "" : "s"} cancelled.`;
    });
  }
</script>

<section class="jobs">
  <h1>Jobs</h1>

  {#if !overview}
    <p class="muted">Loading…</p>
  {:else}
    <p class="activity">
      {#if overview.import_running || overview.retrieval_running || overview.running.length > 0}
        Working:
        {#each overview.running as job (job.id)}
          <span class="now">{kindLabels[job.kind]} {job.pub_key ?? `job ${job.id}`}</span>
        {/each}
        {#if overview.running.length === 0}
          <span class="muted">waiting for a turn…</span>
        {/if}
      {:else}
        <span class="muted">Idle.</span>
      {/if}
    </p>

    <table>
      <thead>
        <tr>
          <th>Kind</th>
          <th>Pending</th>
          <th>Running</th>
          <th>Failed</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each overview.counts as c (c.kind)}
          <tr>
            <td>{kindLabels[c.kind]}</td>
            <td>{c.pending}</td>
            <td>{c.running}</td>
            <td class:failed={c.failed > 0}>{c.failed}</td>
            <td class="actions">
              <button onclick={() => handleRetryAll(c.kind)} disabled={working || c.failed === 0}>
                Retry all failed
              </button>
              {#if cancellable.includes(c.kind)}
                {#if confirmingCancel === c.kind}
                  <button class="danger" onclick={() => handleCancel(c.kind)} disabled={working}>
                    Confirm: cancel {c.pending}
                  </button>
                  <button onclick={() => (confirmingCancel = null)} disabled={working}>Keep</button>
                {:else}
                  <button onclick={() => (confirmingCancel = c.kind)} disabled={working || c.pending === 0}>
                    Cancel pending
                  </button>
                {/if}
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="note">
      Cancelled retrievals leave their documents "not retrieved". They are queued again when the document is
      validated again, or with "retrieve now" and the Library's bulk retrieval.
    </p>

    {#if status}
      <p class="status">{status}</p>
    {/if}
    {#if errorMessage}
      <p class="error-banner">{errorMessage}</p>
    {/if}

    <h2>Failed ({overview.failed_total})</h2>
    {#if overview.failed.length === 0}
      <p class="muted">No failed jobs.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Kind</th>
            <th>Document</th>
            <th>Error</th>
            <th>Attempts</th>
            <th>Last try</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each overview.failed as job (job.id)}
            <tr>
              <td>{kindLabels[job.kind] ?? job.kind}</td>
              <td class="pub-key">{job.pub_key ?? "—"}</td>
              <td class="error">{job.last_error ?? ""}</td>
              <td>{job.attempts}</td>
              <td class="muted">{job.updated_at.replace("T", " ").replace("Z", "")}</td>
              <td><button onclick={() => handleRetry(job.id)} disabled={working}>Retry</button></td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</section>

<style>
  .jobs {
    padding: 1.5rem;
    max-width: 1100px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  h1,
  h2 {
    margin: 0;
  }
  .activity {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .now {
    font-weight: 600;
  }
  .muted {
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
  td.failed {
    color: var(--danger);
    font-weight: 600;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    justify-content: flex-end;
  }
  .pub-key {
    font-weight: 600;
    white-space: nowrap;
  }
  .error {
    overflow-wrap: anywhere;
    font-size: 0.85rem;
  }
  .note {
    margin: 0;
    color: var(--muted);
    font-size: 0.8rem;
  }
  .status {
    margin: 0;
  }
  .error-banner {
    color: var(--danger);
  }
  .danger {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
