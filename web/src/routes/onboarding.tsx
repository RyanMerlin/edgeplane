/**
 * Onboarding screen — Phase 1 React migration.
 *
 * Data source: GET /api/agent-onboarding.json (typed via schema.gen.ts)
 * Cadence:     no auto-refetch (manifest is static per server config)
 *
 * Svelte parity: web/src/routes/onboarding/+page.svelte
 *   - Configuration pane: endpoint URL input + live manifest URL display
 *   - Manifest preview pane: JSON pretty-print of the full OnboardingManifest
 *   - Top bar: Regenerate + Copy actions
 *   - Endpoint auto-detected from window.location.origin on mount
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useToastStore } from '@/stores/toast';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

// ── Generated schema types ─────────────────────────────────────────────────────

type OnboardingManifest = components['schemas']['OnboardingManifest'];

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/onboarding')({
  component: OnboardingPage,
});

// ── Helpers ───────────────────────────────────────────────────────────────────

function defaultEndpoint(): string {
  if (typeof window !== 'undefined') return window.location.origin;
  return 'https://edgeplane.edgeplaneai.app';
}

// ── Main page ──────────────────────────────────────────────────────────────────

// Named export for direct use in tests (avoids router context requirement)
export function OnboardingPage() {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const [endpoint, setEndpoint] = useState<string>(defaultEndpoint);

  // Derive the manifest URL from the current endpoint input
  const manifestUrl = `${endpoint.replace(/\/$/, '')}/api/agent-onboarding.json`;

  // ── Manifest query ─────────────────────────────────────────────────────────
  const manifestQuery = useQuery<OnboardingManifest>({
    queryKey: queryKeys.onboarding.manifest(),
    queryFn: () => unwrap(apiClient.GET('/api/agent-onboarding.json')),
    refetchOnWindowFocus: false,
    retry: 1,
  });

  const manifest = manifestQuery.data;

  // ── Actions ────────────────────────────────────────────────────────────────

  function handleRegenerate() {
    queryClient.invalidateQueries({ queryKey: queryKeys.onboarding.all });
  }

  function handleCopy() {
    const text = manifest ? JSON.stringify(manifest, null, 2) : '';
    if (!text) {
      showToast('No manifest to copy');
      return;
    }
    navigator.clipboard.writeText(text).then(
      () => showToast('Manifest copied to clipboard'),
      () => showToast('Copy failed — check browser permissions'),
    );
  }

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <div className="onboard-page">
      {/* Top bar */}
      <div className="gov-bar">
        <div style={{ marginLeft: 'auto', display: 'flex', gap: '6px', alignItems: 'center' }}>
          <button
            type="button"
            className="ghost"
            onClick={handleRegenerate}
            data-testid="regenerate-btn"
          >
            Regenerate
          </button>
          <button type="button" className="ghost" onClick={handleCopy} data-testid="copy-btn">
            Copy
          </button>
        </div>
      </div>

      {/* Loading */}
      {manifestQuery.isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            ⟳ Loading manifest…
          </p>
        </div>
      )}

      {/* Error */}
      {manifestQuery.isError && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            ✗ Failed to load manifest — {(manifestQuery.error as Error)?.message ?? 'unknown error'}
          </p>
        </div>
      )}

      {/* Content */}
      {!manifestQuery.isLoading && !manifestQuery.isError && (
        <div className="pane-row" style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          {/* Left: configuration */}
          <div className="pane" style={{ width: '280px', flexShrink: 0 }}>
            <div className="pane-header">
              <span className="pane-title">Configuration</span>
            </div>
            <div
              className="pane-body"
              style={{ padding: '10px', display: 'flex', flexDirection: 'column', gap: '8px' }}
            >
              <div>
                <label
                  className="section-label"
                  htmlFor="onboard-endpoint"
                  style={{ display: 'block', marginBottom: '4px' }}
                >
                  Endpoint URL
                </label>
                <input
                  id="onboard-endpoint"
                  value={endpoint}
                  onChange={(e) => setEndpoint(e.target.value)}
                  placeholder="https://edgeplane.example.com"
                  style={{ width: '100%' }}
                  data-testid="endpoint-input"
                />
              </div>
              <div>
                <span className="section-label" style={{ display: 'block', marginBottom: '4px' }}>
                  Manifest URL
                </span>
                <code
                  style={{ fontSize: '11px', color: 'var(--accent)', wordBreak: 'break-all' }}
                  data-testid="manifest-url"
                >
                  {manifestUrl}
                </code>
              </div>
              {manifest && (
                <div style={{ marginTop: '8px' }}>
                  <span className="section-label" style={{ display: 'block', marginBottom: '4px' }}>
                    Instance
                  </span>
                  <div style={{ fontSize: '11px', color: 'var(--muted)' }}>
                    <div>
                      <span className="dim">Name: </span>
                      {manifest.name}
                    </div>
                    <div>
                      <span className="dim">Version: </span>
                      {manifest.version}
                    </div>
                    <div>
                      <span className="dim">Contract: </span>
                      {manifest.integration_contract_version}
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Right: manifest preview */}
          <div className="pane" style={{ flex: 1, minWidth: 0 }}>
            <div className="pane-header">
              <span className="pane-title">Manifest Preview</span>
              {manifestQuery.isFetching && (
                <span className="muted" style={{ fontSize: '10px' }}>
                  ⟳
                </span>
              )}
            </div>
            <div className="pane-body" style={{ padding: '10px' }}>
              {manifest ? (
                <pre style={{ fontSize: '11px' }} data-testid="manifest-json">
                  {JSON.stringify(manifest, null, 2)}
                </pre>
              ) : (
                <p className="muted" data-testid="empty-state">
                  No manifest yet. Click Regenerate.
                </p>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ── Onboarding design-system styles ── */}
      <style>{`
        /* .gov-bar, .gov-title, .pane-row, .pane, .pane-header, .pane-body in app.css */
        .onboard-page {
          display: flex;
          flex-direction: column;
          height: 100%;
          overflow: hidden;
        }

        /* Warm card input — full width with token bg */
        .onboard-page input {
          width: 100%;
          background: var(--input);
          border: 1px solid var(--border);
          border-radius: 6px;
          color: var(--text);
          font-family: inherit;
          font-size: 12px;
          padding: 5px 8px;
        }
        .onboard-page input:focus {
          outline: none;
          border-color: var(--accent);
        }
      `}</style>
    </div>
  );
}
