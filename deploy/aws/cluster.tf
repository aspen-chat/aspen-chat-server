module "cluster" {
  source = "./modules/cluster"

  name               = var.name
  region             = var.region
  image_repositories = var.image_repositories
  image_tag          = var.image_tag
  log_retention_days = var.log_retention_days
  log_roles          = ["api", "worker", "release", "nats", "voice", "restore"]
  execution_secrets = {
    for kind, keys in local.execution_secrets : kind => [for key in keys : aws_secretsmanager_secret.config[key].arn]
  }
}

locals {
  # The configuration files each kind of task may read, by `local.configs` key. The servers'
  # file holds the database's and NATS's passwords, so nothing but the servers may read it; each
  # voice server reads its own file alone.
  execution_secrets = merge(
    { server = ["aspen-toml"], nats = ["nats-conf"], restore = ["database-password"] },
    { for name in keys(var.voice_servers) : "voice-${name}" => ["voice-${name}"] },
  )

  # No servers run until there is an image to run.
  deployed = var.image_tag != null

  runtime_platform = {
    operating_system_family = "LINUX"
    cpu_architecture        = var.cpu_architecture
  }
}
