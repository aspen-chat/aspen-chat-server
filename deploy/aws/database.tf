resource "random_password" "database" {
  length = 40
  # Letters and digits alone, so it needs no escaping in `database_url`.
  special = false
}

resource "aws_db_subnet_group" "this" {
  name       = var.name
  subnet_ids = aws_subnet.private[*].id
}

resource "aws_db_instance" "this" {
  identifier     = var.name
  engine         = "postgres"
  engine_version = var.database.engine_version
  instance_class = var.database.instance_class

  db_name  = "aspen"
  username = "aspen"
  password = random_password.database.result

  allocated_storage     = var.database.allocated_storage
  max_allocated_storage = var.database.max_allocated_storage
  storage_type          = "gp3"
  storage_encrypted     = true

  db_subnet_group_name   = aws_db_subnet_group.this.name
  vpc_security_group_ids = [local.sg["database"]]
  multi_az               = var.database.multi_az
  publicly_accessible    = false

  backup_retention_period    = var.database.backup_retention_days
  auto_minor_version_upgrade = true
  deletion_protection        = var.database.deletion_protection
  skip_final_snapshot        = !var.database.deletion_protection
  final_snapshot_identifier  = "${var.name}-final"
}

# The authorities RDS's certificates in this region come from, which no system trust store
# holds: `database_url` checks the database's certificate against them (sslmode=verify-full).
data "http" "rds_ca" {
  url = "https://truststore.pki.rds.amazonaws.com/${var.region}/${var.region}-bundle.pem"

  lifecycle {
    postcondition {
      condition     = self.status_code == 200 && startswith(self.response_body, "-----BEGIN CERTIFICATE-----")
      error_message = "Could not fetch RDS's certificate authorities for ${var.region}."
    }
  }
}

locals {
  rds_ca_path  = "/etc/aspen/rds-ca.pem"
  database_url = "postgres://${aws_db_instance.this.username}:${random_password.database.result}@${aws_db_instance.this.address}:${aws_db_instance.this.port}/${aws_db_instance.this.db_name}?sslmode=verify-full&sslrootcert=${local.rds_ca_path}"
}
