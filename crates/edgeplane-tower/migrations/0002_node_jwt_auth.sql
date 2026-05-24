-- 0002_node_jwt_auth.sql — node JWT revocation table
--
-- Tracks issued node JWTs by JTI for revocation. The table is intentionally
-- sparse — only revoked tokens need entries. Validation checks signature +
-- exp in-process, then confirms the JTI exists and is not revoked here.
--
-- Retention: rows where expires_at < NOW() can be GC'd safely — a revoked
-- token cannot be replayed after its expiry regardless.

CREATE TABLE public.nodetoken (
    jti        TEXT        PRIMARY KEY,
    node_id    TEXT        NOT NULL REFERENCES public.runtimenode(id) ON DELETE CASCADE,
    revoked    BOOLEAN     NOT NULL DEFAULT false,
    revoked_at TIMESTAMP   WITHOUT TIME ZONE,
    issued_at  TIMESTAMP   WITHOUT TIME ZONE NOT NULL,
    expires_at TIMESTAMP   WITHOUT TIME ZONE NOT NULL
);

CREATE INDEX idx_nodetoken_node_id ON public.nodetoken (node_id);
CREATE INDEX idx_nodetoken_revoked ON public.nodetoken (revoked) WHERE revoked = true;
