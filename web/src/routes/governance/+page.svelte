<script lang="ts">
  import { createQuery } from '@tanstack/svelte-query';
  import { useAuthState } from '$lib/stores/auth-state.svelte';
  import { queryKeys } from '$lib/queryKeys';
  import { fetchPolicy, fetchGovernanceEvents } from '$lib/api';

  const auth = useAuthState();

  const policyQuery = createQuery(() => ({
    queryKey: queryKeys.governance.policy(),
    queryFn: () => fetchPolicy(auth.currentToken || undefined),
    enabled: auth.isLoggedIn
  }));

  const policyEventsQuery = createQuery(() => ({
    queryKey: queryKeys.governance.events(),
    queryFn: () => fetchGovernanceEvents(auth.currentToken || undefined),
    enabled: auth.isLoggedIn
  }));
</script>

<div class="glass-panel">
  <div class="grid">
    <section class="glass-panel">
      <h4>Active Policy</h4>
      <pre>{policyQuery.data ? JSON.stringify(policyQuery.data, null, 2) : 'Loading...'}</pre>
    </section>
    <section class="glass-panel">
      <h4>Policy Events</h4>
      <ul>
        {#each (policyEventsQuery.data ?? []) as evt}
          <li class="muted">[{(evt as { level?: string }).level}] {(evt as { message?: string }).message}</li>
        {:else}
          <li>No events yet.</li>
        {/each}
      </ul>
    </section>
  </div>
</div>
