resource "azurerm_postgresql_flexible_server" "edgeplane" {
  name                = var.server_name
  location            = var.location
  resource_group_name = var.resource_group_name
  administrator_login = var.admin_login
  administrator_login_password = var.admin_password
  sku_name            = var.sku_name

  storage {
    storage_mb = var.storage_mb
  }

  network {
    delegated_subnet_id = var.subnet_id
  }

  backup {
    geo_redundant_backup = "Enabled"
  }

  high_availability {
    mode = "ZoneRedundant"
  }
}
