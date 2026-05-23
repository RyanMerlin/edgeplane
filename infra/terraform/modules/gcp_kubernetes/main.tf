resource "google_container_cluster" "edgeplane" {
  name     = var.cluster_name
  location = var.region
  project  = var.project

  network    = var.network
  subnetwork = var.subnetwork

  remove_default_node_pool = false

  node_config {
    machine_type = var.node_vm_type
  }

  initial_node_count = var.node_count

  workload_identity_config {
    workload_pool = "${var.project}.svc.id.goog"
  }
}
