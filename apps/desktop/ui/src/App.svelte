<!--
  Application root. It creates the application state (module, layout,
  shortcut registry) and loads the backend version.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import AppShell from './components/shell/AppShell.svelte';
  import { createAppState } from './lib/appState';
  import { appVersion, callBackend, isTauri } from './lib/backend';
  import { detectPlatform } from './lib/shortcuts/keys';
  import { toasts } from './lib/stores/toasts.svelte';

  const { modules, layout, library, registry, dispose } = createAppState({
    platform: detectPlatform(typeof navigator === 'undefined' ? undefined : navigator),
    toasts,
  });
  onDestroy(dispose);

  let version = $state('');

  $effect(() => {
    if (!isTauri()) {
      version = 'browser preview';
      return;
    }
    void callBackend('Read the application version', appVersion).then((v) => {
      if (v !== undefined) {
        version = `v${v}`;
      }
    });
  });
</script>

<AppShell {modules} {layout} {library} {registry} {toasts} {version} />
