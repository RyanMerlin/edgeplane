terraform {
  backend "s3" {
    bucket         = var.backend_bucket
    key            = "edgeplane/aws.tfstate"
    region         = var.region
    dynamodb_table = var.backend_dynamodb
  }
}
