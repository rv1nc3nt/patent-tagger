<script lang="ts">
  import ImportScreen from "./lib/ImportScreen.svelte";
  import LibraryScreen from "./lib/LibraryScreen.svelte";
  import MetricsScreen from "./lib/MetricsScreen.svelte";
  import ReviewScreen from "./lib/ReviewScreen.svelte";
  import SettingsScreen from "./lib/SettingsScreen.svelte";

  type View = "import" | "review" | "metrics" | "library" | "settings";
  let view = $state<View>("import");

  const tabs: { id: View; label: string }[] = [
    { id: "import", label: "Import" },
    { id: "review", label: "Review" },
    { id: "metrics", label: "Metrics" },
    { id: "library", label: "Library" },
    { id: "settings", label: "Settings" },
  ];
</script>

<main>
  <nav class="tabs">
    {#each tabs as tab (tab.id)}
      <button class:active={view === tab.id} onclick={() => (view = tab.id)}>{tab.label}</button>
    {/each}
  </nav>

  {#if view === "import"}
    <ImportScreen />
  {:else if view === "review"}
    <ReviewScreen />
  {:else if view === "metrics"}
    <MetricsScreen />
  {:else if view === "library"}
    <LibraryScreen />
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
