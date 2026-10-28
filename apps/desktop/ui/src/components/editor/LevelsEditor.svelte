<!--
  The Levels editor of an adjustment layer: a channel select and five sliders
  (input black, gamma, input white, output black, output white).

  Input and output points show as 0..255 values. A slider drag is one undo
  gesture. The rules for valid values are in `lib/editor/levels.ts`.
-->
<script lang="ts">
  import {
    identityChannel,
    isIdentityChannel,
    LEVELS_CHANNELS,
    setChannelField,
    withChannel,
    type LevelsAdjustment,
    type LevelsChannelName,
    type LevelsField,
  } from '../../lib/editor/levels';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import EditorSlider from './EditorSlider.svelte';

  interface Props {
    editor: EditorStore;
    layerId: number;
    adjustment: LevelsAdjustment;
  }

  let { editor, layerId, adjustment }: Props = $props();

  const LABELS: Readonly<Record<LevelsChannelName, string>> = {
    master: 'Master',
    red: 'Red',
    green: 'Green',
    blue: 'Blue',
  };

  let channel = $state<LevelsChannelName>('master');
  /** The dragged adjustment. It stays until all edits are done. */
  let draft = $state<LevelsAdjustment | null>(null);

  $effect(() => {
    if (editor.idle) {
      draft = null;
    }
  });

  const current = $derived(draft ?? adjustment);
  const values = $derived(current[channel]);

  function send(next: LevelsAdjustment): void {
    draft = next;
    editor.liveEdit(`adjustment:${layerId}`, { type: 'setAdjustment', layerId, adjustment: next });
  }

  /** Sets a field. `scale` converts the shown value to the stored value. */
  function setField(field: LevelsField, shown: number, scale: number): void {
    send(withChannel(current, channel, setChannelField(current[channel], field, shown / scale)));
  }

  function resetChannel(): void {
    send(withChannel(current, channel, identityChannel()));
    editor.endGesture();
  }

  function resetField(field: LevelsField): void {
    setField(field, identityChannel()[field], 1);
    editor.endGesture();
  }

  const toShown = (v: number): number => Math.round(v * 255);
</script>

<div class="levels" role="group" aria-label="Levels">
  <div class="channels" role="group" aria-label="Levels channel">
    {#each LEVELS_CHANNELS as name (name)}
      <button type="button" class="channel {name}" aria-pressed={channel === name} onclick={() => (channel = name)}
        >{LABELS[name]}</button
      >
    {/each}
  </div>
  <EditorSlider
    label="Input black"
    value={toShown(values.inputBlack)}
    min={0}
    max={254}
    onchange={(v) => setField('inputBlack', v, 255)}
    onend={() => editor.endGesture()}
    onreset={() => resetField('inputBlack')}
  />
  <EditorSlider
    label="Gamma"
    value={values.gamma}
    min={0.01}
    max={10}
    step={0.01}
    decimals={2}
    onchange={(v) => setField('gamma', v, 1)}
    onend={() => editor.endGesture()}
    onreset={() => resetField('gamma')}
  />
  <EditorSlider
    label="Input white"
    value={toShown(values.inputWhite)}
    min={1}
    max={255}
    onchange={(v) => setField('inputWhite', v, 255)}
    onend={() => editor.endGesture()}
    onreset={() => resetField('inputWhite')}
  />
  <EditorSlider
    label="Output black"
    value={toShown(values.outputBlack)}
    min={0}
    max={255}
    onchange={(v) => setField('outputBlack', v, 255)}
    onend={() => editor.endGesture()}
    onreset={() => resetField('outputBlack')}
  />
  <EditorSlider
    label="Output white"
    value={toShown(values.outputWhite)}
    min={0}
    max={255}
    onchange={(v) => setField('outputWhite', v, 255)}
    onend={() => editor.endGesture()}
    onreset={() => resetField('outputWhite')}
  />
  <div class="actions">
    <button type="button" disabled={isIdentityChannel(values)} onclick={resetChannel}
      >Reset {LABELS[channel].toLowerCase()}</button
    >
  </div>
</div>

<style>
  .channels {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-4);
  }
  button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  .channel {
    flex: 1;
  }
  .channel[aria-pressed='true'] {
    border-color: var(--color-accent);
  }
  button:disabled {
    opacity: 0.5;
  }
  .actions {
    padding: var(--space-3) var(--space-4);
  }
</style>
