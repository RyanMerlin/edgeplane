output "connection_name" {
  value = google_sql_database_instance.edgeplane.connection_name
}

output "instance_id" {
  value = google_sql_database_instance.edgeplane.id
}

output "ip_address" {
  value = google_sql_database_instance.edgeplane.ip_address[0].ip_address
}
