<script lang="ts">
  import CredentialsPanel from "./lib/CredentialsPanel.svelte";
  import ImportScreen from "./lib/ImportScreen.svelte";
  import ReviewScreen from "./lib/ReviewScreen.svelte";

  let view = $state<"import" | "review">("import");
</script>

<main>
  <nav class="tabs">
    <button class:active={view === "import"} onclick={() => (view = "import")}>Import</button>
    <button class:active={view === "review"} onclick={() => (view = "review")}>Review</button>
  </nav>

  {#if view === "import"}
    <div class="top">
      <CredentialsPanel />
    </div>
    <ImportScreen />
  {:else}
    <ReviewScreen />
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
  .top {
    max-width: 640px;
    margin: 1rem auto 0;
    padding: 0 1.5rem;
  }
</style>
