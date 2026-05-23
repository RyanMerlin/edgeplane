resource "aws_secretsmanager_secret" "edgeplane" {
  name = var.secret_name
}
