terraform {
  backend "gcs" {
    bucket = var.backend_bucket
    prefix = "edgeplane/gcp"
  }
}
