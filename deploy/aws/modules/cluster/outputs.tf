output "id" {
  value = aws_ecs_cluster.this.id
}

output "name" {
  value = aws_ecs_cluster.this.name
}

output "log_group" {
  value = aws_cloudwatch_log_group.this.name
}

output "repositories" {
  description = "Where to push each of the Dockerfile's targets."
  value       = { for target, repository in aws_ecr_repository.image : target => repository.repository_url }
}

output "image" {
  description = "The image each target runs from. With no tag it names an image that does not exist, which nothing may be started from."
  value = {
    for target, repository in aws_ecr_repository.image :
    target => "${repository.repository_url}:${coalesce(var.image_tag, "none")}"
  }
}

output "log_configuration" {
  description = "A container definition's `logConfiguration` by role: one log stream per task, named <role>/<container>/<task id>."
  value = {
    for role in var.log_roles : role => {
      logDriver = "awslogs"
      options = {
        awslogs-group         = aws_cloudwatch_log_group.this.name
        awslogs-region        = var.region
        awslogs-stream-prefix = role
      }
    }
  }
}

output "execution_role_arn" {
  # Read through the policies, so whatever starts a task waits for the role to be able to.
  value      = aws_iam_role.execution.arn
  depends_on = [aws_iam_role_policy_attachment.execution, aws_iam_role_policy.execution_config]
}
