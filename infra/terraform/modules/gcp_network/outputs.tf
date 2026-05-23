output "network_id" {
  value = google_compute_network.edgeplane.id
}

output "subnetwork_id" {
  value = google_compute_subnetwork.edgeplane.id
}
