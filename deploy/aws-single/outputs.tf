output "public_url" {
  value = local.public_url
}

output "address" {
  description = "The machine's address, which the deployment's names point at."
  value       = aws_eip.this.public_ip
}

output "instance_id" {
  description = "For a shell on the machine: aws ssm start-session --target <id>"
  value       = aws_instance.this.id
}

output "voice_server_id" {
  value = random_uuid.voice.result
}

output "run_task" {
  description = "Runs an operator command against the deployment, as a task of its own: append the command, such as `aspen-chat-server admin grant <username>`."
  value = join(" ", [
    "python3 ${path.module}/../aws/run_task.py",
    "--region ${var.region}",
    "--cluster ${module.cluster.name}",
    "--task-definition ${aws_ecs_task_definition.this["release"].family}",
    "--log-group ${module.cluster.log_group}",
    "--log-prefix release",
    "--",
  ])
}
