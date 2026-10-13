output "public_url" {
  value = local.public_url
}

output "image_repositories" {
  description = "Where to push the images built from deploy/docker/Dockerfile, each with its target's name."
  value       = { for target, repository in aws_ecr_repository.image : target => repository.repository_url }
}

output "voice_servers" {
  description = "Each voice server's id in the registry, its signalling address, and the address it takes media on."
  value = {
    for name, _ in var.voice_servers : name => {
      id      = random_uuid.voice[name].result
      url     = "https://${name}.${var.domain}"
      address = aws_eip.voice[name].public_ip
    }
  }
}

output "run_task" {
  description = "Runs an operator command against the deployment, as a task of its own: append the command, such as `aspen-chat-server admin grant <username>`."
  value = join(" ", [
    "python3 ${path.module}/run_task.py",
    "--region ${var.region}",
    "--cluster ${aws_ecs_cluster.this.name}",
    "--task-definition ${aws_ecs_task_definition.server["release"].family}",
    "--subnets ${join(",", aws_subnet.public[*].id)}",
    "--security-group ${local.sg["server"]}",
    "--log-group ${aws_cloudwatch_log_group.this.name}",
    "--log-prefix release",
    "--",
  ])
}
