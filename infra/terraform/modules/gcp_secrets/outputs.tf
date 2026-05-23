output "secret_name" {
  value = google_secret_manager_secret.edgeplane.name
}

output "secret_id" {
  value = google_secret_manager_secret.edgeplane.secret_id
}
