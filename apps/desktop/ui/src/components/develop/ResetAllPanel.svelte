<!--
  Footer of the Develop right column: one button that sets every develop
  section (white balance, tone, tone curve, sharpening, crop) to its
  default. It is disabled when the settings already equal the defaults.
-->
<script lang="ts">
  import type { DevelopStore } from '../../lib/stores/develop.svelte';

  interface Props {
    develop: DevelopStore;
  }

  let { develop }: Props = $props();

  const disabled = $derived(develop.settings === null || develop.atDefaults);
</script>

<div class="footer">
  <button type="button" {disabled} title="Reset all develop settings of this photo" onclick={() => develop.resetAll()}>
    Reset all
  </button>
</div>

<style>
  .footer {
    padding: var(--space-3) var(--space-4);
  }
  button {
    width: 100%;
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  button:disabled {
    opacity: 0.5;
  }
</style>
