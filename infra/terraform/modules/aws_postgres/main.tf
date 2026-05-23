resource "aws_db_subnet_group" "edgeplane" {
  name       = "${var.name_prefix}-subnet-group"
  subnet_ids = var.subnet_ids
}

resource "aws_db_instance" "edgeplane" {
  identifier              = var.instance_identifier
  engine                  = "postgres"
  engine_version          = "15.4"
  instance_class          = var.instance_class
  allocated_storage       = var.storage_gb
  name                    = var.database_name
  username                = var.username
  password                = var.password
  db_subnet_group_name    = aws_db_subnet_group.edgeplane.name
  skip_final_snapshot     = true
  multi_az                = true
  storage_encrypted       = true
  publicly_accessible     = false
  vpc_security_group_ids  = var.security_group_ids
}
