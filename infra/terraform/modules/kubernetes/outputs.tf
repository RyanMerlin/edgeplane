output "cluster_id" {
  value = azurerm_kubernetes_cluster.edgeplane.id
}

output "kube_admin_config_raw" {
  value     = azurerm_kubernetes_cluster.edgeplane.kube_admin_config_raw
  sensitive = true
}
