/**
 * DeleteNodeButton — destructive per-node action with a staged confirmation flow.
 *
 * Backend contract: DELETE /api/runtime/nodes/{node_id}?force=<bool>
 *   200 → { deleted, node_id, detached_agents, revoked_tokens }
 *   409 → { detail, assigned_agents }  (refused — agents still assigned)
 *   403 → not your node; 404 → node not found.
 *
 * UX mirrors the staged inline confirm used in ApprovalPrompt (Reject → Confirm reject):
 *   1. "Delete node" → first confirm ("Confirm delete").
 *   2. Confirm calls DELETE without force. On 409 the bar escalates to a second,
 *      explicit "Force delete (detaches N agents)" confirm; only that click sends
 *      ?force=true. On 200 we toast the detach/revoke counts, invalidate the node
 *      list + detail, and navigate back to /nodes so the deleted row disappears.
 *
 * The generated openapi-fetch DELETE op types every response as `content?: never`
 * (bodies are described, not schema'd in openapi.json), so we use the hand-written
 * `api.delete` wrapper — it parses the JSON body and surfaces it as ApiError.body on
 * non-2xx, which is how we read the 409 `assigned_agents` count.
 */

import { ApiError, api } from '@/lib/api/http';
import { queryKeys } from '@/lib/queryKeys';
import { useToastStore } from '@/stores/toast';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';

interface DeleteNodeResult {
  deleted: boolean;
  node_id: string;
  detached_agents: number;
  revoked_tokens: number;
}

interface ConflictBody {
  detail: string;
  assigned_agents: number;
}

interface DeleteNodeButtonProps {
  nodeId: string;
  nodeName: string;
}

type Stage = 'idle' | 'confirm' | 'force';

export function DeleteNodeButton({ nodeId, nodeName }: DeleteNodeButtonProps) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const [stage, setStage] = useState<Stage>('idle');
  // assigned-agent count surfaced from a 409 response; gates the force confirm.
  const [assignedAgents, setAssignedAgents] = useState<number | null>(null);

  const mutation = useMutation({
    mutationFn: ({ force }: { force: boolean }) =>
      api.delete<DeleteNodeResult>(
        `/runtime/nodes/${encodeURIComponent(nodeId)}${force ? '?force=true' : ''}`,
      ),
    onSuccess: (result) => {
      showToast(
        `Node ${nodeName} deleted — detached ${result.detached_agents} agent(s), revoked ${result.revoked_tokens} token(s)`,
      );
      queryClient.invalidateQueries({ queryKey: queryKeys.nodes.all });
      // Detail row no longer exists — return to the list.
      navigate({ to: '/nodes' });
    },
    onError: (err: unknown) => {
      if (err instanceof ApiError && err.status === 409) {
        // Agents still assigned — escalate to the explicit force confirmation.
        const body = err.body as Partial<ConflictBody> | null;
        const count = typeof body?.assigned_agents === 'number' ? body.assigned_agents : 0;
        setAssignedAgents(count);
        setStage('force');
        return;
      }
      // 403 / 404 / anything else — surface via the shared toast pattern.
      const msg =
        err instanceof ApiError
          ? err.status === 403
            ? `Not permitted to delete node ${nodeName}`
            : err.status === 404
              ? `Node ${nodeName} not found`
              : err.message
          : 'Failed to delete node';
      showToast(msg);
      reset();
    },
  });

  function reset() {
    setStage('idle');
    setAssignedAgents(null);
    mutation.reset();
  }

  const busy = mutation.isPending;

  if (stage === 'idle') {
    return (
      <button
        type="button"
        data-testid="delete-node-btn"
        onClick={() => setStage('confirm')}
        style={dangerButtonStyle}
      >
        Delete node
      </button>
    );
  }

  return (
    <div
      data-testid="delete-node-confirm"
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        padding: '10px 12px',
        background: 'var(--err-bg)',
        border: '1px solid var(--err-border)',
        borderRadius: 6,
        fontSize: 13,
        maxWidth: 420,
      }}
    >
      {stage === 'confirm' ? (
        <span style={{ color: 'var(--text)' }}>
          Delete node <strong>{nodeName}</strong>? This revokes its tokens and removes it from the
          fleet.
        </span>
      ) : (
        <span data-testid="delete-node-force-prompt" style={{ color: 'var(--err)' }}>
          Node <strong>{nodeName}</strong> has {assignedAgents} assigned agent(s). Force delete will
          detach them and delete the node anyway.
        </span>
      )}

      <div style={{ display: 'flex', gap: 6 }}>
        {stage === 'confirm' ? (
          <button
            type="button"
            data-testid="delete-node-confirm-btn"
            disabled={busy}
            onClick={() => mutation.mutate({ force: false })}
            style={dangerButtonStyle}
          >
            {busy ? 'Deleting…' : 'Confirm delete'}
          </button>
        ) : (
          <button
            type="button"
            data-testid="delete-node-force-btn"
            disabled={busy}
            onClick={() => mutation.mutate({ force: true })}
            style={dangerButtonStyle}
          >
            {busy ? 'Deleting…' : `Force delete (detaches ${assignedAgents})`}
          </button>
        )}
        <button
          type="button"
          data-testid="delete-node-cancel-btn"
          disabled={busy}
          onClick={reset}
          style={ghostButtonStyle}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

const dangerButtonStyle: React.CSSProperties = {
  padding: '5px 12px',
  fontSize: 12,
  fontWeight: 510,
  background: 'var(--err-bg)',
  color: 'var(--err)',
  border: '1px solid var(--err-border)',
  borderRadius: 5,
  cursor: 'pointer',
  fontFamily: 'var(--font)',
};

const ghostButtonStyle: React.CSSProperties = {
  padding: '5px 12px',
  fontSize: 12,
  fontWeight: 510,
  background: 'var(--raised)',
  color: 'var(--text-2)',
  border: '1px solid var(--border-subtle)',
  borderRadius: 5,
  cursor: 'pointer',
  fontFamily: 'var(--font)',
};
