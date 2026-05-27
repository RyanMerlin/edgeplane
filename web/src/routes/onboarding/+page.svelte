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

<div class="onboard-page">

  <div class="onboard-bar">
    <span class="onboard-title">Onboarding</span>
    <span class="muted" style="font-size:11px;">Agent enrollment manifest</span>
    <div style="margin-left:auto; display:flex; gap:6px; align-items:center;">
      <button class="ghost" onclick={loadManifest}>Regenerate</button>
      <button class="ghost" onclick={() => navigator.clipboard.writeText(onboardingManifest || '')}>Copy</button>
    </div>
  </div>

  <div class="pane-row" style="flex:1; min-height:0;">

    <div class="pane" style="width:280px; flex-shrink:0;">
      <div class="pane-header"><span class="pane-title">Configuration</span></div>
      <div class="pane-body" style="padding:10px; display:flex; flex-direction:column; gap:8px;">
        <div>
          <label class="section-label" for="onboard-endpoint">Endpoint URL</label>
          <input id="onboard-endpoint" bind:value={onboardingEndpoint} placeholder="https://edgeplane.example.com" style="width:100%;" />
        </div>
        <div>
          <span class="section-label">Manifest URL</span>
          <code style="font-size:11px; color:var(--accent); word-break:break-all;">{manifestUrl || '—'}</code>
        </div>
      </div>
    </div>

    <div class="pane" style="flex:1; min-width:0;">
      <div class="pane-header"><span class="pane-title">Manifest Preview</span></div>
      <div class="pane-body" style="padding:10px;">
        <pre style="font-size:11px;">{onboardingManifest || 'No manifest yet. Click Regenerate.'}</pre>
      </div>
    </div>

  </div>

</div>

<style>
  .onboard-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .onboard-bar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .onboard-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
</style>
