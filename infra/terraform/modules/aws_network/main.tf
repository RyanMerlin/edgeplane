resource "aws_vpc" "edgeplane" {
  cidr_block           = var.cidr_block
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = var.tags
}

resource "aws_subnet" "edgeplane" {
  vpc_id     = aws_vpc.edgeplane.id
  cidr_block = var.subnet_cidr
  availability_zone = var.availability_zone
  tags       = var.tags
}
