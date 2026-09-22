<script lang="ts">
  import CredentialsPanel from "./lib/CredentialsPanel.svelte";
  import ImportScreen from "./lib/ImportScreen.svelte";
</script>

<main>
  <div class="top">
    <CredentialsPanel />
  </div>
  <ImportScreen />
</main>

<style>
  .top {
    max-width: 640px;
    margin: 1rem auto 0;
    padding: 0 1.5rem;
  }
</style>
