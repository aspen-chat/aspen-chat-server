# What every container of a deployment shares: the ECS cluster, its log group, and the role ECS
# starts its tasks with.

resource "aws_ecs_cluster" "this" {
  name = var.name
}

resource "aws_cloudwatch_log_group" "this" {
  name              = "/aspen/${var.name}"
  retention_in_days = var.log_retention_days
}

# What ECS itself does for a task before it starts: pull its image, read its configuration
# file, and write its logs. What a task may do once it runs is its own role's. Each kind of task
# has a role of its own that may read only its own configuration, so a task whose host is taken
# over (a voice server's, which faces the internet on its own machine) yields only what that
# task already had.
resource "aws_iam_role" "execution" {
  for_each = var.execution_secrets
  name     = "${var.name}-${each.key}-exec"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "execution" {
  for_each   = var.execution_secrets
  role       = aws_iam_role.execution[each.key].name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

resource "aws_iam_role_policy" "execution_config" {
  for_each = var.execution_secrets
  name     = "read-configuration"
  role     = aws_iam_role.execution[each.key].id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "secretsmanager:GetSecretValue"
      Resource = each.value
    }]
  })
}
