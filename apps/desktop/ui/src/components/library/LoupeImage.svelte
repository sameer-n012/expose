<!--
  The loupe image: the thumbnail of the photo (scaled to fit) while the
  preview session opens, then the preview canvas on top of it after the
  first presented frame. It shows the saved develop settings and never
  changes them. If the preview fails, the thumbnail stays and a note
  shows the reason.
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import type { Photo } from '../../lib/catalog';
  import type { FrameTransport } from '../../lib/develop/transport';
  import { LoupeStore, type LoupeApi } from '../../lib/library/loupe.svelte';
  import { thumbnailUrl } from '../../lib/library/thumbnail';
  import { describeError } from '../../lib/stores/toasts.svelte';
  import PreviewCanvas from '../develop/PreviewCanvas.svelte';

  interface Props {
    photo: Photo;
    /** Test hook: the backend calls. */
    api?: LoupeApi;
    /** Test hook: the frame transport. */
    transport?: FrameTransport;
  }

  let { photo, api, transport }: Props = $props();

  // The store is made once; `api` is read once, at creation.
  const loupe = new LoupeStore(untrack(() => api));
  let framePhoto: number | null = $state(null);
  let frameError: string | null = $state(null);

  $effect(() => {
    const id = photo.id;
    framePhoto = null;
    frameError = null;
    untrack(() => void loupe.show(id));
  });

  onDestroy(() => loupe.dispose());

  const drawn = $derived(framePhoto !== null && framePhoto === loupe.photoId);
  const note = $derived(loupe.error ?? frameError);
</script>

<div class="loupe-image" data-testid="loupe-image">
  <img class="thumb" src={thumbnailUrl(photo)} alt={photo.fileName} data-testid="loupe-thumbnail" />
  {#if loupe.settings !== null && loupe.session !== null}
    <div class="canvas" class:hidden={!drawn} data-testid="loupe-canvas">
      <PreviewCanvas
        session={loupe.session}
        settings={loupe.settings}
        {...transport === undefined ? {} : { transport }}
        onframe={() => (framePhoto = loupe.photoId)}
        onerror={(error) => (frameError = describeError(error))}
      />
    </div>
  {/if}
  {#if note !== null}
    <p class="note" role="status" data-testid="loupe-error">The preview could not be shown: {note}</p>
  {/if}
</div>

<style>
  .loupe-image {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
  }
  .thumb {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .canvas {
    position: absolute;
    inset: 0;
  }
  .canvas.hidden {
    visibility: hidden;
  }
  .note {
    position: absolute;
    left: var(--space-3);
    bottom: var(--space-3);
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-danger);
  }
</style>
