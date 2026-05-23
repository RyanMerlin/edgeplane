resource "azurerm_key_vault" "edgeplane" {
  name                       = var.key_vault_name
  location                   = var.location
  resource_group_name        = var.resource_group_name
  tenant_id                  = var.tenant_id
  sku_name                   = "standard"
  purge_protection_enabled   = var.purge_protection
  soft_delete_retention_days = 7

  access_policy {
    tenant_id = var.tenant_id
    object_id = var.creator_object_id

    key_permissions    = ["get", "list", "encrypt", "decrypt"]
    secret_permissions = ["get", "list", "set"]
  }
}
