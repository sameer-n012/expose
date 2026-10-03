<!--
  Placeholder panel content: a list of collapsible sections.
  Real module panels replace this component later.
-->
<script lang="ts">
  interface Props {
    sections: readonly string[];
  }

  let { sections }: Props = $props();
</script>

{#each sections as title (title)}
  <details class="section" open>
    <summary>{title}</summary>
    <p class="placeholder">No content yet.</p>
  </details>
{/each}

<style>
  .section {
    border-bottom: 1px solid var(--color-border);
  }
  summary {
    padding: var(--space-3) var(--space-4);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-strong);
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: default;
  }
  summary:hover {
    color: var(--color-text);
  }
  .placeholder {
    margin: 0;
    padding: 0 var(--space-4) var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--color-text-faint);
  }
</style>
