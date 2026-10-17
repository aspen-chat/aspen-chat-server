# Each release's one-off work, done during the apply and before the servers are replaced:
# the order docs/operators/installing/upgrading.md gives. It runs again whenever the release
# task's definition changes, which a new image, a changed aspen.toml, or a changed voice
# server does; both of its commands change nothing when there is nothing to change.
resource "terraform_data" "release" {
  count            = local.deployed ? 1 : 0
  triggers_replace = [aws_ecs_task_definition.server["release"].arn]

  provisioner "local-exec" {
    command = "python3 ${path.module}/run_task.py"

    environment = {
      ASPEN_RUN_REGION          = var.region
      ASPEN_RUN_CLUSTER         = module.cluster.name
      ASPEN_RUN_TASK_DEFINITION = aws_ecs_task_definition.server["release"].arn
      ASPEN_RUN_SUBNETS         = join(",", aws_subnet.public[*].id)
      ASPEN_RUN_SECURITY_GROUP  = local.sg["server"]
      ASPEN_RUN_LOG_GROUP       = module.cluster.log_group
      ASPEN_RUN_LOG_PREFIX      = "release"
    }
  }

  depends_on = [
    aws_db_instance.this,
    aws_ecs_service.nats,
    aws_vpc_security_group_ingress_rule.internal,
    aws_vpc_security_group_egress_rule.all,
    aws_route_table_association.public,
    aws_secretsmanager_secret_version.config,
    aws_iam_role_policy.server_media,
  ]
}
