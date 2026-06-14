export const queryKeys = {
  onboarding: {
    all: ['onboarding'] as const,
    manifest: () => [...queryKeys.onboarding.all, 'manifest'] as const,
  },
  ai: {
    all: ['ai'] as const,
    sessions: () => [...queryKeys.ai.all, 'sessions'] as const,
    session: (id: string) => [...queryKeys.ai.all, 'session', id] as const,
    turn: (sessionId: string, turnId: number) =>
      [...queryKeys.ai.all, 'turn', sessionId, turnId] as const,
  },
  explorer: {
    all: ['explorer'] as const,
    tree: () => [...queryKeys.explorer.all, 'tree'] as const,
    node: (type: string, id: string) => [...queryKeys.explorer.all, 'node', type, id] as const,
  },
  domains: {
    all: ['domains'] as const,
    northstar: (domainId: string) => [...queryKeys.domains.all, domainId, 'northstar'] as const,
    brief: (domainId: string, missionId: string) =>
      [...queryKeys.domains.all, domainId, 'missions', missionId, 'brief'] as const,
  },
  nodes: {
    all: ['nodes'] as const,
    list: () => [...queryKeys.nodes.all, 'list'] as const,
    detail: (nodeId: string) => [...queryKeys.nodes.all, 'detail', nodeId] as const,
  },
  governance: {
    all: ['governance'] as const,
    policy: () => [...queryKeys.governance.all, 'policy'] as const,
    versions: () => [...queryKeys.governance.all, 'versions'] as const,
    events: () => [...queryKeys.governance.all, 'events'] as const,
  },
  agents: {
    all: ['agents'] as const,
    list: () => [...queryKeys.agents.all, 'list'] as const,
    detail: (agentId: string) => [...queryKeys.agents.all, 'detail', agentId] as const,
  },
  jobs: {
    all: ['jobs'] as const,
    list: () => [...queryKeys.jobs.all, 'list'] as const,
    detail: (id: number) => [...queryKeys.jobs.all, 'detail', id] as const,
  },
};
