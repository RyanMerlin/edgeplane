# Cloud Integration Blueprint

This document captures the architecture blueprint discussed for deploying MissionControl into Azure first, then reusing the same platform contract in GCP and AWS. It focuses on the platform baseline (networking, managed Kubernetes, secrets, monitoring, CI/CD, and backing services) so the MissionControl application can remain configuration-driven via the existing `MC_*` environment variables.

## Platform Contract (shared across clouds)

- **Private networking**: hub-and-spoke/VPC model with dedicated load-balanced ingress, egress filtering, and mutual TLS for cross-zone traffic.
- **Managed Kubernetes**: AKS/GKE/EKS clusters with node pools for API workloads, autoscaling, and admission controls that enforce namespace/certs/secrets policies.
- **Managed PostgreSQL**: Regional service (Azure Database for PostgreSQL, Cloud SQL, Amazon RDS) with private endpoints and replicas for HA.
- **Object storage**: MinIO hosted in the cluster (or cloud-native S3-compatible store) reachable via the `MC_OBJECT_STORAGE_*` variables; authentication happens through vault-backed credentials and policies restrict the `missions/<mission>/klusters/<kluster>/` prefixes.
- **MQTT**: Stateful broker deployed in Kubernetes with persistence, with the option to replace it using cloud-managed services later.
- **Secrets and identities**: External Secrets, Azure Key Vault/Google Secret Manager/Secrets Manager, and workload identities avoiding embedded credentials in `.env` files.
- **Observability**: OpenTelemetry collectors feeding Azure Monitor/Cloud Monitoring/CloudWatch plus cargo Prometheus endpoints for ingest, along with alerting/ runbooks per failure domain.
- **CI/CD and promotion**: GitHub Actions pipeline runs Terraform plan/apply per environment, triggers Kubernetes deployment (helm/kustomize), and validates `/healthz`, `/readyz`, and `/mcp/health` before marking a release.
- **Disaster recovery**: Automated backups for Postgres/MinIO, documented restore steps, and smoke-test scripts that run after failover.

## Azure Baseline (Priority 1)

### Networking
- Terraform module builds Azure hub VNet, spoke for MissionControl, private endpoint for Azure Database for PostgreSQL, and service endpoints for AKS.
- NVA or firewall controls enforce outbound egress policies and restrict inbound to internal load balancers/front-door.

### Compute
- AKS cluster with two node pools (system + missioncontrol). Nodes join private network with managed identities for pull secrets, Key Vault access, and blob storage.
- Helm/Kustomize deploys `backend` and `integrations/mc` services with Kubernetes Services/Ingress, secrets mounted from External Secrets Operator, and ConfigMaps for `MC_*` values.

### Data plane
- Azure Database for PostgreSQL (flexible server) provisioned with custom parameter group, SSL-only connections, and geo-redundant backup; Terraform exports connection strings consumed via Key Vault secrets.
- MinIO deployment uses persistent volumes backed by managed disk, configured to present `mc`-compatible endpoint for `MC_OBJECT_STORAGE_ENDPOINT` (e.g., `http://minio.missioncontrol.svc.cluster.local`).
- Mosquitto MQTT broker runs as StatefulSet with PVCs; optionally, Azure IoT Hub rules can be introduced later.

### Security & controls
- Key Vault holds `MC_TOKEN`, `SLACK_BOT_TOKEN`, and object storage credentials; Azure AD service principals grant AKS workload identity limited scope.
- Azure Policy enforces resource tags, private endpoint usage, and disk encryption; RBAC roles restrict terraform operators vs. runtime admin.

### Observability & ops
- Azure Monitor autoscraps metrics from AKS + Postgres + MinIO, with alerts for high latency, crash looping, and API health checks hitting MissionControl endpoints.
- Add Runbooks for failover tests, credential rotation, MinIO bucket checks, Terraform state drift detection, and security scan remediation.

### CI/CD
- GitHub Actions workflow using `azure/cli`, `hashicorp/setup-terraform`, and `azure/k8s-deploy` to plan/apply infra and push Helm charts; gating job runs smoke tests before `prod` stack is marked healthy.
- Terraform state stored in Azure Storage with locking (e.g., via `azurerm_storage_account` + `azurerm_storage_container`).

## GCP Baseline (Priority 2)

- **Terraform reuse**: Re-point provider block to `google` and reuse the shared module signatures (VPC, subnets, GKE integration, private services access). Document the handful of provider-specific overrides (e.g., `master_authorized_networks`, `workload_identity_pool`).
- **Networking**: VPC + private GKE cluster with Cloud NAT for egress, Private Service Connect for Cloud SQL, and firewall rules matching the Azure baseline.
- **Compute**: GKE autopilot or standard node pools with node autoscaling, shielded GKE nodes, and Cloud Build triggers for Kubernetes deployments that reference the same Helm charts.
- **Data**: Cloud SQL Postgres instance with automated backups; secrets synced to Secret Manager and mounted via External Secrets Operator. MinIO and MQTT follow the same Kubernetes deployment with PVs on regional `pd-ssd` disks.
- **Ops**: Cloud Monitoring dashboards mirroring Azure alerts; Cloud Logging sinks to audit log buckets. Add documented steps for verifying readiness after applying Terraform and re-running smoke tests.

## AWS Baseline (Priority 3)

- Mirror Terraform modules with the AWS provider: VPC, EKS, RDS Postgres, Secrets Manager, and CloudWatch.
- Use private EKS cluster with IAM Roles for Service Accounts (IRSA), node groups for API workloads, and security groups that enforce ingress/egress policies from Azure/GCP baselines.
- RDS multi-AZ Postgres with automated backups; object storage can remain MinIO on EKS, optionally reachable via AWS Application Load Balancer if exposing external access.
- CloudWatch collects metrics/logs, with alerting on Pod failures similar to earlier waves.
- CI/CD pipeline clones the same Terraform repo, using `aws-actions/configure-aws-credentials` and `aws eks update-kubeconfig` before applying manifests.

## Acceptance Criteria

- Terraform modules validate across Azure, GCP, and AWS providers with provider-specific workspaces and state backends.
- Managed Kubernetes deployments satisfy `/healthz`, `/readyz`, and `/mcp/health` with secrets injected from each cloud's secret store and the `MC_*` variables unchanged.
- Observability runbooks show alerts for database failure, storage issues, and MQTT connectivity; each runbook has a response strategy and verification steps.
- CI/CD runs ensure smoke tests pass before marking stages healthy, and recoverable backups exist for Postgres and MinIO with documented restore commands.
