/// Lightweight carrier that keeps agent identifiers and approval hints together.
#[derive(Clone, Debug, Default)]
pub struct AgentContext {
    pub agent_id: Option<String>,
    pub runtime_session_id: Option<String>,
    pub profile_name: Option<String>,
}

impl AgentContext {
    pub fn new(
        agent_id: Option<String>,
        runtime_session_id: Option<String>,
        profile_name: Option<String>,
    ) -> Self {
        Self {
            agent_id,
            runtime_session_id,
            profile_name,
        }
    }
}
