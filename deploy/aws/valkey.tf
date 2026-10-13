resource "random_password" "valkey" {
  length  = 40
  special = false
}

resource "aws_elasticache_subnet_group" "this" {
  name       = var.name
  subnet_ids = aws_subnet.private[*].id
}

# ElastiCache evicts by default. Aspen's Valkey must never: an evicted key is a rate limit
# count or a sign-in in progress (docs/operators/installing/1-services.md).
resource "aws_elasticache_parameter_group" "this" {
  name   = var.name
  family = "valkey8"

  parameter {
    name  = "maxmemory-policy"
    value = "noeviction"
  }
}

resource "aws_elasticache_replication_group" "this" {
  replication_group_id = var.name
  description          = "Aspen ${var.name}: rate limits, sign-ins in progress, mailed codes"

  engine               = "valkey"
  engine_version       = "8.0"
  node_type            = var.valkey_node_type
  num_cache_clusters   = 1
  parameter_group_name = aws_elasticache_parameter_group.this.name
  port                 = 6379

  subnet_group_name  = aws_elasticache_subnet_group.this.name
  security_group_ids = [local.sg["valkey"]]

  # The servers refuse to send a password to another machine without TLS.
  transit_encryption_enabled = true
  transit_encryption_mode    = "required"
  auth_token                 = random_password.valkey.result
  at_rest_encryption_enabled = true

  # Everything in it expires on its own and nothing is restored from it.
  snapshot_retention_limit = 0
}

locals {
  valkey_url = "rediss://:${random_password.valkey.result}@${aws_elasticache_replication_group.this.primary_endpoint_address}:6379"
}
