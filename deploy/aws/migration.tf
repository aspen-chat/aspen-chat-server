# Arriving from a box (README.md#moving-a-box-here): a task that restores the box's database,
# copied into this deployment's bucket, into this one's empty database. migrate.py runs it,
# before this deployment's first release.

locals {
  restore_file = "/dump/database.dump"
  # All of it or none: a database that already holds tables, or a dump that fails part of the
  # way, leaves the database as it was. The deployment's own user comes to own everything.
  restore_command = ["pg_restore", "--single-transaction", "--exit-on-error", "--no-owner", "--no-acl"]
  # Writes RDS's authorities where libpq is told to look for them, then runs the command.
  restore_entry_point = ["sh", "-ec", "printf '%s' \"$RDS_CA\" > \"$PGSSLROOTCERT\"\nexec \"$@\"\n", "aspen"]
}

# What the restore task itself may do: read the one dump.
resource "aws_iam_role" "restore" {
  name = "${var.name}-restore"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy" "restore" {
  name = "get-the-dump"
  role = aws_iam_role.restore.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "s3:GetObject"
      Resource = module.media.migration_dump.arn
    }]
  })
}

resource "aws_ecs_task_definition" "restore" {
  family                   = "${var.name}-restore"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = 1024
  memory                   = 2048
  execution_role_arn       = module.cluster.execution_role_arn
  task_role_arn            = aws_iam_role.restore.arn

  runtime_platform {
    operating_system_family = local.runtime_platform.operating_system_family
    cpu_architecture        = local.runtime_platform.cpu_architecture
  }

  # The dump, between the container that fetches it and the one that restores it.
  volume {
    name = "dump"
  }

  container_definitions = jsonencode([
    {
      name             = "fetch"
      image            = "public.ecr.aws/aws-cli/aws-cli:latest"
      essential        = false
      command          = ["s3", "cp", module.media.migration_dump.uri, local.restore_file]
      environment      = [{ name = "AWS_REGION", value = var.region }]
      mountPoints      = [{ sourceVolume = "dump", containerPath = dirname(local.restore_file) }]
      logConfiguration = module.cluster.log_configuration["restore"]
    },
    {
      name       = "aspen"
      image      = "public.ecr.aws/docker/library/postgres:${var.database.engine_version}-alpine"
      essential  = true
      dependsOn  = [{ containerName = "fetch", condition = "SUCCESS" }]
      entryPoint = local.restore_entry_point
      command    = concat(local.restore_command, ["--dbname=${aws_db_instance.this.db_name}", local.restore_file])
      environment = [
        { name = "PGHOST", value = aws_db_instance.this.address },
        { name = "PGPORT", value = tostring(aws_db_instance.this.port) },
        { name = "PGUSER", value = aws_db_instance.this.username },
        { name = "PGSSLMODE", value = "verify-full" },
        { name = "PGSSLROOTCERT", value = "/tmp/rds-ca.pem" },
        { name = "RDS_CA", value = data.http.rds_ca.response_body },
      ]
      secrets          = [{ name = "PGPASSWORD", valueFrom = local.config_secret["database-password"] }]
      mountPoints      = [{ sourceVolume = "dump", containerPath = dirname(local.restore_file) }]
      logConfiguration = module.cluster.log_configuration["restore"]
    },
  ])
}

output "migration" {
  description = "What migrate.py reads to move a box here."
  value = {
    name           = var.name
    region         = var.region
    domain         = var.domain
    cluster        = module.cluster.name
    log_group      = module.cluster.log_group
    subnets        = join(",", aws_subnet.public[*].id)
    security_group = local.sg["server"]
    restore_task   = aws_ecs_task_definition.restore.family
    release_task   = aws_ecs_task_definition.server["release"].family
    voice_servers  = keys(var.voice_servers)
    bucket         = module.media.bucket
    dump           = module.media.migration_dump.uri
    load_balancer  = aws_lb.this.dns_name
  }
}
