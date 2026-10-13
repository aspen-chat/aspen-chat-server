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

locals {
  # No servers run until there is an image to run.
  deployed = var.image_tag != null
  image = {
    for target, repository in aws_ecr_repository.image :
    target => "${repository.repository_url}:${coalesce(var.image_tag, "none")}"
  }

  runtime_platform = {
    operating_system_family = "LINUX"
    cpu_architecture        = var.cpu_architecture
  }

  # One log stream per task, named <role>/<container>/<task id>.
  log_configuration = {
    for role in ["api", "worker", "release", "nats", "voice"] : role => {
      logDriver = "awslogs"
      options = {
        awslogs-group         = aws_cloudwatch_log_group.this.name
        awslogs-region        = var.region
        awslogs-stream-prefix = role
      }
    }
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
      Resource = [for secret in aws_secretsmanager_secret.config : secret.arn]
    }]
  })
}
