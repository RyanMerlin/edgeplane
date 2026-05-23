terraform {
  required_version = ">= 1.4"

  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = ">= 3.0"
    }
  }
}

provider "azurerm" {
  features        = {}
  subscription_id = var.subscription_id
  tenant_id       = var.tenant_id
}

resource "azurerm_resource_group" "edgeplane" {
  name     = var.resource_group_name
  location = var.location
  tags     = var.tags
}

module "network" {
  source               = "../../modules/network"
  resource_group_name  = azurerm_resource_group.edgeplane.name
  location             = var.location
  virtual_network_name = "edgeplane-vnet"
  subnet_prefix        = var.subnet_prefix
  tags                 = var.tags
}

module "kubernetes" {
  source              = "../../modules/kubernetes"
  resource_group_name = azurerm_resource_group.edgeplane.name
  location            = var.location
  cluster_name        = "edgeplane-aks"
  dns_prefix          = "edgeplane"
  node_count          = var.kubernetes_node_count
  node_vm_size        = var.kubernetes_node_size
}

module "postgres" {
  source          = "../../modules/postgres"
  location        = var.location
  resource_group_name = azurerm_resource_group.edgeplane.name
  server_name     = "edgeplane-postgres"
  admin_login     = var.postgres_admin_login
  admin_password  = var.postgres_admin_password
  subnet_id       = module.network.subnet_id
}

module "secrets" {
  source              = "../../modules/secrets"
  key_vault_name      = "edgeplane-keyvault"
  location            = var.location
  resource_group_name = azurerm_resource_group.edgeplane.name
  tenant_id           = var.tenant_id
  creator_object_id   = var.creator_object_id
}
