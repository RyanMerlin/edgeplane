resource "google_sql_database_instance" "edgeplane" {
  name             = var.instance_name
  database_version = "POSTGRES_15"
  region           = var.region
  project          = var.project

  settings {
    tier = var.tier

    disk_autoresize   = true
    disk_size         = var.disk_size
    disk_type         = "PD_SSD"
    activation_policy = "ALWAYS"

    ip_configuration {
      private_network = var.network_self_link
      ipv4_enabled    = false
    }
  }

  deletion_protection = false
}
