module "cluster" {
  source = "./modules/cluster"

  name               = var.name
  region             = var.region
  image_repositories = var.image_repositories
  image_tag          = var.image_tag
  log_retention_days = var.log_retention_days
  log_roles          = ["api", "worker", "release", "nats", "voice", "restore"]
  config_secret_arns = [for secret in aws_secretsmanager_secret.config : secret.arn]
}

locals {
  # No servers run until there is an image to run.
  deployed = var.image_tag != null

  runtime_platform = {
    operating_system_family = "LINUX"
    cpu_architecture        = var.cpu_architecture
  }
}
