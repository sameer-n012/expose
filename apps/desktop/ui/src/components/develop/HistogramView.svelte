<!--
  The RGB histogram of the Develop module, as three translucent channel
  areas (drawn with the screen blend mode, so overlaps look additive).

  Scaling: the height of a bin is sqrt(count / peak), so small counts stay
  visible. The peak is the largest count in the bins 1 to 254 of all
  channels. Thus a spike at 0 or 255 does not flatten the rest; such a
  spike is cut at the top. The two triangles show clipping (see
  `clipping`). Without a histogram the panel shows nothing but its title.
-->
<script lang="ts">
  import { clipping, type Histogram } from '../../lib/develop/histogram';

  interface Props {
    histogram: Histogram | null;
  }

  let { histogram }: Props = $props();

  const WIDTH = 240;
  const HEIGHT = 96;

  /** The points of a closed area for one channel. */
  function areaPoints(bins: Uint32Array, peak: number): string {
    const parts: string[] = [`0,${HEIGHT}`];
    for (let i = 0; i < 256; i++) {
      const scaled = Math.min(1, Math.sqrt((bins[i] ?? 0) / peak));
      const x = (i / 255) * WIDTH;
      parts.push(`${x.toFixed(1)},${(HEIGHT - scaled * HEIGHT).toFixed(1)}`);
    }
    parts.push(`${WIDTH},${HEIGHT}`);
    return parts.join(' ');
  }

  function peakOf(h: Histogram): number {
    let peak = 1;
    for (const bins of [h.r, h.g, h.b]) {
      for (let i = 1; i < 255; i++) {
        peak = Math.max(peak, bins[i] ?? 0);
      }
    }
    return peak;
  }

  const shape = $derived.by(() => {
    if (histogram === null) {
      return null;
    }
    const peak = peakOf(histogram);
    return {
      r: areaPoints(histogram.r, peak),
      g: areaPoints(histogram.g, peak),
      b: areaPoints(histogram.b, peak),
      clip: clipping(histogram),
    };
  });
</script>

<details class="section" open>
  <summary>Histogram</summary>
  <div class="body">
    {#if shape !== null}
      <div class="plot" data-testid="histogram">
        <svg viewBox="0 0 {WIDTH} {HEIGHT}" width={WIDTH} height={HEIGHT} role="img" aria-label="RGB histogram">
          <polygon class="red" points={shape.r} />
          <polygon class="green" points={shape.g} />
          <polygon class="blue" points={shape.b} />
        </svg>
        <span
          class="clip left"
          class:lit={shape.clip.shadows}
          data-testid="clip-shadows"
          role="img"
          aria-label={shape.clip.shadows ? 'Shadows are clipped' : 'No shadow clipping'}
        ></span>
        <span
          class="clip right"
          class:lit={shape.clip.highlights}
          data-testid="clip-highlights"
          role="img"
          aria-label={shape.clip.highlights ? 'Highlights are clipped' : 'No highlight clipping'}
        ></span>
      </div>
    {/if}
  </div>
</details>

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
  .body:empty {
    display: none;
  }
  .plot {
    position: relative;
    width: 240px;
    max-width: calc(100% - 2 * var(--space-4));
    margin: 0 var(--space-4) var(--space-3);
    background: var(--color-bg-app);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
  }
  polygon {
    mix-blend-mode: screen;
    fill-opacity: 0.7;
  }
  .red {
    fill: var(--color-label-red);
  }
  .green {
    fill: var(--color-label-green);
  }
  .blue {
    fill: var(--color-label-blue);
  }
  .clip {
    position: absolute;
    top: var(--space-1);
    width: 0;
    height: 0;
    border-top: 8px solid var(--color-border-strong);
    opacity: 0.6;
  }
  .left {
    left: var(--space-1);
    border-right: 8px solid transparent;
  }
  .right {
    right: var(--space-1);
    border-left: 8px solid transparent;
  }
  .lit {
    border-top-color: var(--color-text);
    opacity: 1;
  }
</style>
