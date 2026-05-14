-- Add home_mission_id (permanent anchor) and current_mission_id (active attachment)
-- to the agent table. Both are nullable FKs to mission(id).
-- home_mission_id is set once at registration and never cleared.
-- current_mission_id follows the agent's active context; detach resets it to home.

ALTER TABLE agent
    ADD COLUMN home_mission_id    TEXT REFERENCES mission(id),
    ADD COLUMN current_mission_id TEXT REFERENCES mission(id);

CREATE INDEX agent_home_mission_idx    ON agent(home_mission_id);
CREATE INDEX agent_current_mission_idx ON agent(current_mission_id);
