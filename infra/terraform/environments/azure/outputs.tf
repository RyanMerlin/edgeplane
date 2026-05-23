output "resource_group_name" {
  value = azurerm_resource_group.edgeplane.name
}

output "postgres_hostname" {
  value = module.postgres.postgres_hostname
}

output "kubernetes_credentials" {
  value     = module.kubernetes.kube_admin_config_raw
  sensitive = true
}

output "key_vault_uri" {
  value = module.secrets.key_vault_uri
}
