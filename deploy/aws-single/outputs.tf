output "public_url" {
  value = "https://${var.domain}"
}

output "addresses" {
  description = "The machine's addresses, which the deployment's names point at."
  value       = [aws_eip.this.public_ip, local.ipv6_address]
}

output "instance_id" {
  description = "For a shell on the machine: aws ssm start-session --target <id>"
  value       = aws_instance.this.id
}

output "on_machine" {
  description = "Runs a command on the machine and prints what it wrote: append it, such as `aspen-box run -- aspen-chat-server admin grant <username>`."
  value       = "python3 ${path.module}/on_machine.py --region ${var.region} --instance ${aws_instance.this.id} --"
}
