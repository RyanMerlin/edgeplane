<script lang="ts">
  import { onMount } from 'svelte';
  import { showToast } from '$lib/stores/toast';
  import { browser } from '$app/environment';

  let onboardingEndpoint = $state('');
  let onboardingManifest = $state('');
  let manifestUrl = $state('');

  function defaultOnboardingEndpoint() {
    if (!browser) return 'https://edgeplane.edgeplaneai.app';
    return window.location.origin;
  }

  async function syncOnboardingEndpoint() {
    const localUrl = `${defaultOnboardingEndpoint().replace(/\/$/, '')}/agent-onboarding.json`;
    try {
      const res = await fetch(localUrl);
      if (!res.ok) return;
      const manifest = await res.json();
      onboardingEndpoint = String(manifest?.generated_for_base_url || manifest?.endpoints?.ui || '').replace(/\/ui\/$/, '');
    } catch { /* ignore */ }
  }

  async function loadManifest() {
    const normalized = (onboardingEndpoint || defaultOnboardingEndpoint()).replace(/\/$/, '');
    manifestUrl = `${normalized}/agent-onboarding.json`;
    try {
      const res = await fetch(manifestUrl);
      if (!res.ok) throw new Error(`Manifest fetch failed (${res.status})`);
      const manifest = await res.json();
      onboardingManifest = JSON.stringify(manifest, null, 2);
    } catch (err) {
      onboardingManifest = '';
      showToast(err instanceof Error ? err.message : 'Failed to load onboarding manifest');
    }
  }

  onMount(() => {
    onboardingEndpoint = defaultOnboardingEndpoint();
    syncOnboardingEndpoint().finally(() => loadManifest());
  });
</script>

<div class="glass-panel">
  <h3>Agent Onboarding</h3>
  <label>Endpoint<input bind:value={onboardingEndpoint} placeholder="https://edgeplane.example.com" /></label>
  <div class="onboarding-actions">
    <button class="ghost" onclick={loadManifest}>Regenerate Manifest</button>
    <button class="ghost" onclick={() => navigator.clipboard.writeText(onboardingManifest || '')}>Copy</button>
  </div>
  <div class="grid">
    <section class="glass-panel"><h4>Manifest URL</h4><code>{manifestUrl || 'fetch to generate'}</code></section>
    <section class="glass-panel"><h4>Manifest Preview</h4><pre>{onboardingManifest || 'No manifest yet.'}</pre></section>
  </div>
</div>
