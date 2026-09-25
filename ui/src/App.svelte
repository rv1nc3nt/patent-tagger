<script lang="ts">
  import { jobOverview, type JobOverview } from "./lib/api";
  import ImportScreen from "./lib/ImportScreen.svelte";
  import JobsScreen from "./lib/JobsScreen.svelte";
  import LibraryScreen from "./lib/LibraryScreen.svelte";
  import MetricsScreen from "./lib/MetricsScreen.svelte";
  import ReviewScreen from "./lib/ReviewScreen.svelte";
  import SettingsScreen from "./lib/SettingsScreen.svelte";
  import TagsScreen from "./lib/TagsScreen.svelte";
  import ViewScreen from "./lib/ViewScreen.svelte";

  type View = "import" | "review" | "view" | "tags" | "metrics" | "library" | "jobs" | "settings";
  let view = $state<View>("import");
  // The document open in the View tab, kept while other tabs are shown.
  let viewDocId = $state<number | null>(null);

  function openInView(docId: number) {
    viewDocId = docId;
    view = "view";
  }

  const tabs: { id: View; label: string }[] = [
    { id: "import", label: "Import" },
    { id: "review", label: "Review" },
    { id: "view", label: "View" },
    { id: "tags", label: "Tags" },
    { id: "metrics", label: "Metrics" },
    { id: "library", label: "Library" },
    { id: "jobs", label: "Jobs" },
    { id: "settings", label: "Settings" },
  ];

  // Job queue state, polled for the Jobs tab's label and screen: every 2 s
  // while the Jobs screen is open, every 5 s otherwise.
  let jobs = $state<JobOverview | null>(null);

  async function refreshJobs() {
    try {
      jobs = await jobOverview();
    } catch {
      // Keep the last known state; the Jobs screen shows command errors.
    }
  }

  $effect(() => {
    const interval = view === "jobs" ? 2000 : 5000;
    refreshJobs();
    const timer = setInterval(refreshJobs, interval);
    return () => clearInterval(timer);
  });

  function tabLabel(id: View, label: string): string {
    if (id !== "jobs" || !jobs) return label;
    const parts = [];
    if (jobs.active_total > 0) parts.push(String(jobs.active_total));
    if (jobs.failed_total > 0) parts.push(`${jobs.failed_total} failed`);
    return parts.length > 0 ? `${label} (${parts.join(", ")})` : label;
  }
</script>

<main>
  <nav class="tabs">
    {#each tabs as tab (tab.id)}
      <button class:active={view === tab.id} onclick={() => (view = tab.id)}>{tabLabel(tab.id, tab.label)}</button>
    {/each}
  </nav>

  {#if view === "import"}
    <ImportScreen />
  {:else if view === "review"}
    <ReviewScreen onView={openInView} />
  {:else if view === "view"}
    <ViewScreen bind:docId={viewDocId} />
  {:else if view === "tags"}
    <TagsScreen />
  {:else if view === "metrics"}
    <MetricsScreen />
  {:else if view === "library"}
    <LibraryScreen onView={openInView} />
  {:else if view === "jobs"}
    <JobsScreen overview={jobs} refresh={refreshJobs} />
  {:else}
    <SettingsScreen />
  {/if}
</main>

<style>
  .tabs {
    display: flex;
    gap: 0.5rem;
    padding: 1rem 1.5rem 0;
    max-width: 1100px;
    margin: 0 auto;
  }
  .tabs button {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 0.4rem 0.2rem;
    font-weight: 600;
    color: var(--muted);
  }
  .tabs button.active {
    color: var(--text-h);
    border-bottom-color: var(--accent);
  }
</style>
