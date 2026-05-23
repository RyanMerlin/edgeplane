output "virtual_network_id" {
  value = azurerm_virtual_network.primary.id
}

output "subnet_id" {
  value = azurerm_subnet.edgeplane.id
}
