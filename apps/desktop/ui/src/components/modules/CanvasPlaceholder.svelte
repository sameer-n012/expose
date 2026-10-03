<!--
  Center view placeholder for the Develop and Editor modules: a neutral
  canvas area. The GPU preview surface replaces this later.
-->
<script lang="ts">
  interface Props {
    title: string;
    hint: string;
  }

  let { title, hint }: Props = $props();
</script>

<div class="canvas">
  <div class="frame">
    <div class="title">{title}</div>
    <div class="hint">{hint}</div>
  </div>
</div>

<style>
  .canvas {
    height: 100%;
    display: grid;
    place-items: center;
    padding: var(--space-6);
  }
  .frame {
    width: min(100%, 900px);
    aspect-ratio: 3 / 2;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    background: var(--color-bg-placeholder);
  }
  .title {
    font-size: var(--font-size-xl);
    color: var(--color-text-muted);
  }
  .hint {
    font-size: var(--font-size-sm);
    /* Muted, not faint: faint text is below 4.5:1 on the placeholder gray. */
    color: var(--color-text-muted);
  }
</style>
