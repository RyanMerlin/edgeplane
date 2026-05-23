terraform {
  required_version = ">= 1.4"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0"
    }
  }
}

provider "google" {
  project = var.project
  region  = var.region
  zone    = var.zone
}

module "network" {
  source         = "../../modules/gcp_network"
  project        = var.project
  region         = var.region
  network_name   = "edgeplane-network"
  subnetwork_name = "edgeplane-subnet"
  subnetwork_cidr = var.subnetwork_cidr
}

module "kubernetes" {
  source          = "../../modules/gcp_kubernetes"
  project         = var.project
  region          = var.region
  cluster_name    = "edgeplane-gke"
  network         = module.network.network_id
  subnetwork      = module.network.subnetwork_id
  node_count      = var.kubernetes_node_count
  node_vm_type    = var.kubernetes_node_type
}

module "postgres" {
  source             = "../../modules/gcp_postgres"
  project            = var.project
  region             = var.region
  instance_name      = "edgeplane-sql"
  tier               = var.postgres_tier
  disk_size          = var.postgres_disk_size
  network_self_link  = module.network.network_id
}

module "secrets" {
  source    = "../../modules/gcp_secrets"
  project   = var.project
  secret_id = "edgeplane-secrets"
}
