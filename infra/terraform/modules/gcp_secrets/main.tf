resource "google_secret_manager_secret" "edgeplane" {
  secret_id = var.secret_id
  project   = var.project

  replication {
    automatic = true
  }
}
