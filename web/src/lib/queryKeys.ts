export const queryKeys = {
	ai: {
		all: ['ai'] as const,
		sessions: () => [...queryKeys.ai.all, 'sessions'] as const,
		session: (id: string) => [...queryKeys.ai.all, 'session', id] as const,
		turn: (sessionId: string, turnId: number) =>
			[...queryKeys.ai.all, 'turn', sessionId, turnId] as const
	},
	explorer: {
		all: ['explorer'] as const,
		tree: () => [...queryKeys.explorer.all, 'tree'] as const,
		node: (type: string, id: string) =>
			[...queryKeys.explorer.all, 'node', type, id] as const
	},
	governance: {
		all: ['governance'] as const,
		policy: () => [...queryKeys.governance.all, 'policy'] as const,
		events: () => [...queryKeys.governance.all, 'events'] as const
	},
	evolve: {
		all: ['evolve'] as const,
		mission: (id: string) => [...queryKeys.evolve.all, 'mission', id] as const
	},
	jobs: {
		all: ['jobs'] as const,
		list: () => [...queryKeys.jobs.all, 'list'] as const,
		detail: (id: number) => [...queryKeys.jobs.all, 'detail', id] as const
	}
};
