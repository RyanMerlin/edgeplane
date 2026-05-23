terraform {
  required_version = ">= 1.4"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.0"
    }
  }
}

provider "aws" {
  region  = var.region
  profile = var.profile
}

module "network" {
  source            = "../../modules/aws_network"
  cidr_block        = var.vpc_cidr
  subnet_cidr       = var.subnet_cidr
  availability_zone = var.availability_zone
  tags              = var.tags
}

resource "aws_security_group" "postgres" {
  name        = "edgeplane-postgres"
  description = "Allow internal access to Postgres"
  vpc_id      = module.network.vpc_id
  ingress {
    from_port   = 5432
    to_port     = 5432
    protocol    = "tcp"
    cidr_blocks = [var.vpc_cidr]
  }
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

module "kubernetes" {
  source          = "../../modules/aws_kubernetes"
  cluster_name    = "edgeplane-eks"
  cluster_role_arn = var.cluster_role_arn
  node_role_arn   = var.node_role_arn
  subnet_ids      = [module.network.subnet_id]
  node_count      = var.kubernetes_node_count
  max_size        = var.kubernetes_node_max
  min_size        = var.kubernetes_node_min
}

module "postgres" {
  source              = "../../modules/aws_postgres"
  name_prefix         = "edgeplane"
  instance_identifier = "edgeplane-db"
  database_name       = "edgeplane"
  username            = var.postgres_username
  password            = var.postgres_password
  subnet_ids          = [module.network.subnet_id]
  security_group_ids  = [aws_security_group.postgres.id]
}

module "secrets" {
  source      = "../../modules/aws_secrets"
  secret_name = "edgeplane-secrets"
}
