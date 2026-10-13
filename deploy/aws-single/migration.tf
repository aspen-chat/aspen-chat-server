# Leaving for the larger configuration (../aws/README.md#moving-from-one-machine): a task that
# dumps the database into the bucket, which that deployment takes over and restores from.
# ../aws/migrate_from_single.py runs it.

locals {
  dump_file = "/dump/database.dump"
  # The dump is of the whole database as one consistent moment, whatever is written meanwhile.
  dump_command = ["pg_dump", "--format=custom", "--file=${local.dump_file}", "--host=127.0.0.1", "--username=aspen", "aspen"]
}

# What the export task itself may do: put the one dump, and nothing else in the bucket.
resource "aws_iam_role" "export" {
  name = "${var.name}-export"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy" "export" {
  name = "put-the-dump"
  role = aws_iam_role.export.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["s3:PutObject", "s3:AbortMultipartUpload"]
      Resource = module.media.migration_dump.arn
    }]
  })
}

resource "aws_ecs_task_definition" "export" {
  family                   = "${var.name}-export"
  requires_compatibilities = ["EC2"]
  network_mode             = "host"
  execution_role_arn       = module.cluster.execution_role_arn
  task_role_arn            = aws_iam_role.export.arn

  # The dump, between the container that writes it and the one that uploads it. It is on the
  # machine's own disk, which must have room for it.
  volume {
    name = "dump"
  }

  container_definitions = jsonencode([
    {
      name              = "dump"
      image             = local.containers.postgres.image
      essential         = false
      command           = local.dump_command
      memoryReservation = 128
      secrets           = [{ name = "PGPASSWORD", valueFrom = local.config_secret["database-password"] }]
      mountPoints       = [{ sourceVolume = "dump", containerPath = dirname(local.dump_file) }]
      logConfiguration  = module.cluster.log_configuration["export"]
    },
    {
      name              = "aspen"
      image             = "public.ecr.aws/aws-cli/aws-cli:latest"
      essential         = true
      dependsOn         = [{ containerName = "dump", condition = "SUCCESS" }]
      command           = ["s3", "cp", local.dump_file, module.media.migration_dump.uri]
      memoryReservation = 128
      environment       = [{ name = "AWS_REGION", value = var.region }]
      mountPoints       = [{ sourceVolume = "dump", containerPath = dirname(local.dump_file) }]
      logConfiguration  = module.cluster.log_configuration["export"]
    },
  ])
}

output "handover" {
  description = "What ../aws/migrate_from_single.py reads to move this deployment to the larger configuration."
  value = {
    name            = var.name
    region          = var.region
    domain          = var.domain
    route53_zone_id = var.route53_zone_id
    cluster         = module.cluster.name
    log_group       = module.cluster.log_group
    servers         = [for server in aws_ecs_service.server : server.name]
    export_task     = aws_ecs_task_definition.export.family
    voice_server    = local.voice_server_name
    bucket          = module.media.bucket
    distribution_id = module.media.distribution_id
    # Forgotten rather than destroyed when this deployment is taken down: the storage the
    # larger one goes on using, the name that now points at it, and the data volume, kept as
    # the way back.
    kept = concat(
      [for address in module.media.handed_over : "module.media.${address}"],
      ["aws_route53_record.machine[\"${var.domain}\"]", "aws_ebs_volume.data"],
    )
    data_volume = aws_ebs_volume.data.id
  }
}
