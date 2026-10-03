import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

const target = document.getElementById('app');
if (target === null) {
  throw new Error('The element #app is missing from index.html.');
}

/**
 * Development only: `?bench=transport` runs the frame transport benchmark
 * in place of the application (see `lib/develop/bench.ts`). The module is
 * loaded only in this mode.
 */
async function runBench(root: HTMLElement, params: URLSearchParams): Promise<void> {
  const { runTransportBench } = await import('./lib/develop/bench');
  root.innerHTML = '';
  const output = document.createElement('pre');
  output.style.cssText = 'margin:8px;font:12px monospace;white-space:pre;color:inherit';
  const canvas = document.createElement('canvas');
  canvas.style.cssText = 'max-width:100%;max-height:50vh;display:block;margin:8px';
  root.append(output, canvas);
  const log = (line: string): void => {
    console.info(line);
    output.textContent += `${line}\n`;
  };
  const number = (name: string): number | undefined => {
    const value = Number(params.get(name));
    return Number.isFinite(value) && value > 0 ? value : undefined;
  };
  const frames = number('frames');
  const longEdge = number('long');
  try {
    await runTransportBench({
      canvas,
      log,
      ...(frames === undefined ? {} : { frames }),
      ...(longEdge === undefined ? {} : { longEdge }),
    });
  } catch (error: unknown) {
    console.error('The transport benchmark failed', error);
    log(`FAILED: ${error instanceof Error ? error.message : JSON.stringify(error)}`);
  }
}

const params = new URLSearchParams(window.location.search);
if (params.get('bench') === 'transport') {
  void runBench(target, params);
}

export default params.get('bench') === 'transport' ? undefined : mount(App, { target });
