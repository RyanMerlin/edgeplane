resource "google_compute_network" "edgeplane" {
  name                    = var.network_name
  auto_create_subnetworks = false
  routing_mode            = "GLOBAL"
  project                 = var.project
}

resource "google_compute_subnetwork" "edgeplane" {
  name          = var.subnetwork_name
  ip_cidr_range = var.subnetwork_cidr
  network       = google_compute_network.edgeplane.id
  region        = var.region
  project       = var.project
}
