output "postgres_server_id" {
  value = azurerm_postgresql_flexible_server.edgeplane.id
}

output "postgres_hostname" {
  value = azurerm_postgresql_flexible_server.edgeplane.fqdn
}
