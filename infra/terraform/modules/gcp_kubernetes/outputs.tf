output "cluster_name" {
  value = google_container_cluster.edgeplane.name
}

output "node_pool_id" {
  value = google_container_node_pool.edgeplane.id
}

output "endpoint" {
  value = google_container_cluster.edgeplane.endpoint
}

output "ca_certificate" {
  value = google_container_cluster.edgeplane.master_auth[0].cluster_ca_certificate
}
