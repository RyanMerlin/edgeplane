import { api } from '@/lib/api/http';
import { useMutation } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

export const Route = createFileRoute('/admin/')({
  component: AdminIndexPage,
});

interface JoinTokenResponse {
  id: string;
  node_id: string | null;
  status: string;
  expires_at: string;
  token: string;
}

interface JoinTokenCreate {
  expires_in_seconds: number;
  upgrade_channel: string;
  desired_version: string;
  config: Record<string, unknown>;
}

export function AdminIndexPage() {
  const [expiresIn, setExpiresIn] = useState(3600);
  const [upgradeChannel, setUpgradeChannel] = useState('stable');
  const [copied, setCopied] = useState(false);

  const mutation = useMutation({
    mutationFn: (body: JoinTokenCreate) => api.post<JoinTokenResponse>('/runtime/tokens', body),
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    mutation.reset();
    setCopied(false);
    mutation.mutate({
      expires_in_seconds: Number.isFinite(expiresIn) && expiresIn >= 60 ? expiresIn : 3600,
      upgrade_channel: upgradeChannel,
      desired_version: '',
      config: {},
    });
  };

  const handleCopy = () => {
    if (mutation.data?.token) {
      navigator.clipboard
        .writeText(mutation.data.token)
        .then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 2000);
        })
        .catch(() => {});
    }
  };

  return (
    <div style={{ padding: '16px 24px', maxWidth: 560 }}>
      <div style={{ marginBottom: 16 }}>
        <span
          style={{
            fontSize: 11,
            fontWeight: 590,
            color: 'var(--dim)',
            letterSpacing: '0.06em',
            textTransform: 'uppercase',
          }}
        >
          Admin
        </span>
        <h1
          style={{
            margin: '4px 0 0',
            fontSize: 18,
            fontWeight: 600,
            color: 'var(--text)',
          }}
        >
          Create Join Token
        </h1>
        <p style={{ margin: '4px 0 0', fontSize: 13, color: 'var(--text-2)' }}>
          Generates a one-time token a new node uses to enroll with this EdgePlane instance.
        </p>
      </div>

      <form
        onSubmit={handleSubmit}
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 14,
          padding: 16,
          background: 'var(--frame)',
          border: '1px solid var(--border)',
          borderRadius: 8,
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label
            htmlFor="expires-in"
            style={{ fontSize: 12, fontWeight: 510, color: 'var(--text-2)' }}
          >
            Expires in (seconds)
          </label>
          <input
            id="expires-in"
            type="number"
            min={60}
            value={expiresIn}
            onChange={(e) => setExpiresIn(Number(e.target.value))}
            data-testid="expires-in-seconds"
            style={{
              fontSize: 13,
              padding: '5px 9px',
              background: 'var(--input)',
              color: 'var(--text)',
              border: '1px solid var(--border-subtle)',
              borderRadius: 5,
              fontFamily: 'var(--mono)',
              width: 160,
            }}
          />
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label
            htmlFor="upgrade-channel"
            style={{ fontSize: 12, fontWeight: 510, color: 'var(--text-2)' }}
          >
            Upgrade channel
          </label>
          <input
            id="upgrade-channel"
            type="text"
            value={upgradeChannel}
            onChange={(e) => setUpgradeChannel(e.target.value)}
            data-testid="upgrade-channel"
            style={{
              fontSize: 13,
              padding: '5px 9px',
              background: 'var(--input)',
              color: 'var(--text)',
              border: '1px solid var(--border-subtle)',
              borderRadius: 5,
              fontFamily: 'var(--mono)',
              width: 160,
            }}
          />
        </div>

        <div>
          <button
            type="submit"
            disabled={mutation.isPending}
            style={{
              padding: '6px 16px',
              fontSize: 13,
              fontWeight: 510,
              background: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: 5,
              cursor: mutation.isPending ? 'not-allowed' : 'pointer',
              opacity: mutation.isPending ? 0.7 : 1,
              fontFamily: 'var(--font)',
            }}
          >
            {mutation.isPending ? 'Creating…' : 'Create Token'}
          </button>
        </div>
      </form>

      {mutation.isError && (
        <div
          data-testid="token-error"
          style={{
            marginTop: 14,
            padding: '10px 14px',
            background: 'var(--err-dim, rgba(255,80,80,0.1))',
            border: '1px solid var(--err)',
            borderRadius: 6,
            fontSize: 13,
            color: 'var(--err)',
          }}
        >
          {mutation.error instanceof Error ? mutation.error.message : 'Failed to create token.'}
        </div>
      )}

      {mutation.isSuccess && mutation.data && (
        <div
          data-testid="token-result"
          style={{
            marginTop: 14,
            display: 'flex',
            flexDirection: 'column',
            gap: 10,
          }}
        >
          {/* Warning banner */}
          <div
            style={{
              padding: '8px 12px',
              background: 'var(--warn-dim, rgba(255,180,0,0.12))',
              border: '1px solid var(--warn, #e0a000)',
              borderRadius: 6,
              fontSize: 12,
              color: 'var(--warn, #e0a000)',
              fontWeight: 510,
            }}
          >
            This token is shown only once — copy it now. It cannot be retrieved again.
          </div>

          {/* Token display */}
          <div
            style={{
              padding: 14,
              background: 'var(--frame)',
              border: '1px solid var(--border)',
              borderRadius: 8,
              display: 'flex',
              flexDirection: 'column',
              gap: 12,
            }}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <span
                style={{
                  fontSize: 11,
                  color: 'var(--dim)',
                  fontWeight: 510,
                  textTransform: 'uppercase',
                  letterSpacing: '0.06em',
                }}
              >
                Token
              </span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <code
                  data-testid="token-value"
                  style={{
                    flex: 1,
                    fontSize: 12,
                    fontFamily: 'var(--mono)',
                    color: 'var(--text)',
                    background: 'var(--input)',
                    border: '1px solid var(--border-subtle)',
                    borderRadius: 4,
                    padding: '6px 10px',
                    wordBreak: 'break-all',
                    display: 'block',
                  }}
                >
                  {mutation.data.token}
                </code>
                <button
                  type="button"
                  data-testid="copy-token"
                  onClick={handleCopy}
                  style={{
                    flexShrink: 0,
                    padding: '5px 12px',
                    fontSize: 12,
                    fontWeight: 510,
                    background: copied ? 'var(--ok-dim, rgba(0,200,80,0.12))' : 'var(--raised)',
                    color: copied ? 'var(--ok)' : 'var(--text-2)',
                    border: '1px solid var(--border-subtle)',
                    borderRadius: 5,
                    cursor: 'pointer',
                    fontFamily: 'var(--font)',
                    transition: 'background .15s, color .15s',
                  }}
                >
                  {copied ? 'Copied!' : 'Copy'}
                </button>
              </div>
            </div>

            {/* Metadata */}
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))',
                gap: 8,
              }}
            >
              {[
                { label: 'ID', value: mutation.data.id },
                { label: 'Status', value: mutation.data.status },
                { label: 'Expires at', value: mutation.data.expires_at },
              ].map(({ label, value }) => (
                <div key={label} style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                  <span
                    style={{
                      fontSize: 10,
                      color: 'var(--dim)',
                      textTransform: 'uppercase',
                      letterSpacing: '0.06em',
                      fontWeight: 590,
                    }}
                  >
                    {label}
                  </span>
                  <span
                    style={{
                      fontSize: 12,
                      color: 'var(--text-2)',
                      fontFamily: 'var(--mono)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                    title={value}
                  >
                    {value || '—'}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
