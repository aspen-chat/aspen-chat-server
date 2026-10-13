output "public_url" {
  value = local.public_url
}

output "image_repositories" {
  description = "Where to push the images built from deploy/docker/Dockerfile, each with its target's name."
  value       = module.cluster.repositories
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
    "--cluster ${module.cluster.name}",
    "--task-definition ${aws_ecs_task_definition.server["release"].family}",
    "--subnets ${join(",", aws_subnet.public[*].id)}",
    "--security-group ${local.sg["server"]}",
    "--log-group ${module.cluster.log_group}",
    "--log-prefix release",
    "--",
  ])
}
