resource "azurerm_virtual_network" "primary" {
  name                = var.virtual_network_name
  location            = var.location
  resource_group_name = var.resource_group_name
  address_space       = var.address_space
  tags                = var.tags
}

resource "azurerm_subnet" "edgeplane" {
  name                 = "edgeplane-subnet"
  resource_group_name  = var.resource_group_name
  virtual_network_name = azurerm_virtual_network.primary.name
  address_prefixes     = [var.subnet_prefix]
}
