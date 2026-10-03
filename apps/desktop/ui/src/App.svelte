<script lang="ts">
  import { appVersion, isTauri } from './lib/backend';

  let version = $state<string>('');
  let error = $state<string>('');

  $effect(() => {
    if (!isTauri()) {
      version = 'browser preview';
      return;
    }
    appVersion()
      .then((v) => {
        version = v;
      })
      .catch((e: unknown) => {
        error = e instanceof Error ? e.message : String(e);
      });
  });
</script>

<main>
  <h1>Expose</h1>
  {#if error}
    <p class="error">Backend error: {error}</p>
  {:else}
    <p class="muted">Version {version}</p>
  {/if}
</main>

<style>
  main {
    display: grid;
    place-content: center;
    height: 100%;
    text-align: center;
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: #e5484d;
  }
</style>
