output "db_instance_id" {
  value = aws_db_instance.edgeplane.id
}

output "endpoint" {
  value = aws_db_instance.edgeplane.endpoint
}
