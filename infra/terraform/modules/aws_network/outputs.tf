output "vpc_id" {
  value = aws_vpc.edgeplane.id
}

output "subnet_id" {
  value = aws_subnet.edgeplane.id
}
