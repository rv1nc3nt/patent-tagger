<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { readTextFile } from "@tauri-apps/plugin-fs";
  import {
    importNumbers,
    onImportProgress,
    retryDocument,
    runImportJobs,
    type DocumentOutcome,
    type ImportReport,
  } from "./api";

  let rawInput = $state("");
  let report = $state<ImportReport | null>(null);
  let outcomes = $state<DocumentOutcome[]>([]);
  let isBusy = $state(false);
  let progressDone = $state(0);
  let progressTotal = $state(0);
  let errorMessage = $state("");

  const fetched = $derived(outcomes.filter((o) => o.status === "fetched"));
  const noEnglishAbstract = $derived(outcomes.filter((o) => o.status === "no_english_abstract"));
  const notFound = $derived(outcomes.filter((o) => o.status === "not_found"));
  const errors = $derived(outcomes.filter((o) => o.status === "error"));
  const related = $derived(
    Array.from(new Set(outcomes.flatMap((o) => o.related_pub_keys))).filter(
      (pubKey) => !outcomes.some((o) => o.pub_key === pubKey),
    ),
  );

  function upsertOutcome(outcome: DocumentOutcome) {
    const index = outcomes.findIndex((o) => o.doc_id === outcome.doc_id);
    if (index === -1) {
      outcomes = [...outcomes, outcome];
    } else {
      outcomes = [...outcomes.slice(0, index), outcome, ...outcomes.slice(index + 1)];
    }
    progressDone += 1;
  }

  $effect(() => {
    const unlisten = onImportProgress(upsertOutcome);
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  async function runFetchStage() {
    isBusy = true;
    progressDone = 0;
    progressTotal = report?.imported.length ?? 0;
    try {
      await runImportJobs();
    } catch (err) {
      errorMessage = String(err);
    } finally {
      isBusy = false;
    }
  }

  async function handleImport() {
    errorMessage = "";
    isBusy = true;
    try {
      report = await importNumbers(rawInput);
      rawInput = "";
    } catch (err) {
      errorMessage = String(err);
      isBusy = false;
      return;
    }
    await runFetchStage();
  }

  async function handleChooseFile() {
    const path = await openDialog({
      multiple: false,
      filters: [{ name: "Patent numbers", extensions: ["txt", "csv"] }],
    });
    if (typeof path === "string") {
      rawInput = await readTextFile(path);
    }
  }

  async function handleRetry(docId: number) {
    errorMessage = "";
    try {
      await retryDocument(docId);
      await runFetchStage();
    } catch (err) {
      errorMessage = String(err);
    }
  }
</script>

<section class="import">
  <h1>Import</h1>

  <textarea
    bind:value={rawInput}
    placeholder="Paste publication numbers, one per line…"
    rows="8"
    disabled={isBusy}
  ></textarea>

  <div class="toolbar">
    <button onclick={handleChooseFile} disabled={isBusy}>Choose file…</button>
    <button onclick={handleImport} disabled={isBusy || !rawInput.trim()}>Import</button>
    {#if isBusy}
      <span class="progress">Fetching {progressDone} / {progressTotal}…</span>
    {/if}
  </div>

  {#if errorMessage}
    <p class="error-banner">{errorMessage}</p>
  {/if}

  {#if report}
    <div class="report">
      <h2>Report</h2>

      {@render section("Imported", fetched.map((o) => `${o.pub_key} — ${o.title ?? ""}`))}
      {@render section("Duplicates", report.duplicates)}
      {@render section("Related documents", related)}
      {@render section("No English abstract", noEnglishAbstract.map((o) => o.pub_key))}
      {@render section("Not found", notFound.map((o) => o.pub_key))}
      {@render section("Needs manual number lookup", report.needs_normalisation)}
      {@render section("Could not parse", report.unparseable)}

      {#if errors.length > 0 || notFound.length > 0}
        <div class="section">
          <h3>Errors</h3>
          <ul>
            {#each errors as outcome (outcome.doc_id)}
              <li>
                <span class="pub-key">{outcome.pub_key}</span>
                <span class="detail">{outcome.error}</span>
                <button class="retry" onclick={() => handleRetry(outcome.doc_id)} disabled={isBusy}>
                  Retry
                </button>
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>
  {/if}
</section>

{#snippet section(title: string, items: string[])}
  {#if items.length > 0}
    <div class="section">
      <h3>{title} ({items.length})</h3>
      <ul>
        {#each items as item}
          <li>{item}</li>
        {/each}
      </ul>
    </div>
  {/if}
{/snippet}

<style>
  .import {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 640px;
    margin: 0 auto;
    padding: 1.5rem;
  }
  textarea {
    font-family: inherit;
    padding: 0.5rem;
    resize: vertical;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .progress {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .error-banner {
    color: var(--danger);
  }
  .report {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .section h3 {
    margin-bottom: 0.25rem;
  }
  .section ul {
    margin: 0;
    padding-left: 1.25rem;
  }
  li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .pub-key {
    font-weight: 600;
  }
  .detail {
    color: var(--muted);
    font-size: 0.85rem;
    flex: 1;
  }
  .retry {
    font-size: 0.8rem;
  }
</style>
