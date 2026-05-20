# MissionControl K8s overlay

Deploys the `backend`, `crates/mc`, and supporting services on a managed Kubernetes cluster. It expects:

- a `ConfigMap` containing the `MC_*` environment variables.
- Secrets injected via External Secrets Operator from Key Vault or Secret Manager.
- A `ServiceAccount` with permissions to mount secrets and config maps.

Run `kubectl apply -k infra/k8s/missioncontrol` once Terraform has published the kubeconfig for the target cluster.
