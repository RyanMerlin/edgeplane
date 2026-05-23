# Azure Environment

This directory orchestrates the Azure baseline for Edgeplane. Key responsibilities:

- Wire the shared Terraform modules into an Azure subscription.
- Configure `backend.tf` so Terraform state lives in an Azure Storage account with locking.
- Export outputs for the resource group, Kubernetes credentials, PostgreSQL host, and Key Vault URI.
- Reference the variables defined in `variables.tf` to keep values parameterized.

When applying locally, set the sensitive inputs via CLI or a `.tfvars` file (see `terraform.tfvars.example`).
