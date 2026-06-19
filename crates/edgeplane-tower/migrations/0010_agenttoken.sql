-- Per-agent JWT revocation registry (Seam 2). Mirrors nodetoken.
CREATE TABLE agenttoken (
    jti         character varying NOT NULL PRIMARY KEY,
    agent_id    character varying NOT NULL,
    domain_id   character varying NOT NULL,
    revoked     boolean NOT NULL DEFAULT false,
    expires_at  timestamp without time zone NOT NULL,
    created_at  timestamp without time zone NOT NULL DEFAULT now()
);
CREATE INDEX agenttoken_agent_id_idx ON agenttoken (agent_id);
