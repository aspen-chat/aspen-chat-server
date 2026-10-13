# What every container of a deployment shares: the ECS cluster, its log group, the repositories
# its images are pushed to, and the role ECS starts its tasks with. Both AWS configurations use
# it.

resource "aws_ecs_cluster" "this" {
  name = var.name
}

resource "aws_cloudwatch_log_group" "this" {
  name              = "/aspen/${var.name}"
  retention_in_days = var.log_retention_days
}

resource "aws_ecr_repository" "image" {
  for_each             = toset(["api", "voice"])
  name                 = "${var.name}/${each.key}"
  image_tag_mutability = "IMMUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

# What ECS itself does for a task before it starts: pull its image, read its configuration
# file, and write its logs. The servers hold no AWS permissions of their own.
resource "aws_iam_role" "execution" {
  name = "${var.name}-task-execution"

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
  role       = aws_iam_role.execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

resource "aws_iam_role_policy" "execution_config" {
  name = "read-configuration"
  role = aws_iam_role.execution.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "secretsmanager:GetSecretValue"
      Resource = var.config_secret_arns
    }]
  })
}
