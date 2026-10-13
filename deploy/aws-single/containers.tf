# Everything the deployment runs, each a container on the machine's own network.

module "cluster" {
  source = "../aws/modules/cluster"

  name               = var.name
  region             = var.region
  image_tag          = var.image_tag
  log_retention_days = var.log_retention_days
  log_roles          = concat(keys(local.containers), ["export"])
  config_secret_arns = [for secret in aws_secretsmanager_secret.config : secret.arn]
}

locals {
  # Neither server runs until there is an image to run.
  deployed = var.image_tag != null

  # Each container starts through a script that writes its configuration file from its
  # environment into its working directory, and then runs the command it was given. An operator
  # command given to a release task in its command's place (run_task.py) starts the same way,
  # which is how it finds the deployment's aspen.toml.
  write = {
    for file, variable in {
      "aspen.toml"           = "ASPEN_TOML"
      "voice_server.toml"    = "VOICE_SERVER_TOML"
      "/tmp/nats.conf"       = "NATS_CONF"
      "/etc/caddy/Caddyfile" = "CADDYFILE"
    } :
    file => ["sh", "-ec", "umask 077\nprintf '%s' \"${"$"}${variable}\" > ${file}\nunset ${variable}\nexec \"$@\"\n", "aspen"]
  }

  # `memory` is a reservation in MiB, which ECS places tasks by; none is a limit.
  containers = {
    postgres = {
      # 18 or later, which the migrations need.
      image       = "public.ecr.aws/docker/library/postgres:18-alpine"
      command     = ["postgres", "-c", "listen_addresses=127.0.0.1"]
      environment = { POSTGRES_USER = "aspen", POSTGRES_DB = "aspen" }
      secrets     = { POSTGRES_PASSWORD = "database-password" }
      volumes     = { "/data/postgres" = "/var/lib/postgresql" }
      memory      = 256
      # Long enough for PostgreSQL to finish what it is writing and stop cleanly.
      stop_timeout = 120
    }
    valkey = {
      image = "public.ecr.aws/valkey/valkey:8-alpine"
      # Nothing in it is worth keeping across a restart, and it must never evict
      # (docs/operators/installing/1-services.md).
      command = [
        "valkey-server", "--bind", "127.0.0.1", "--save", "",
        "--maxmemory", "${var.valkey_memory_mb}mb", "--maxmemory-policy", "noeviction",
      ]
      memory = 64
    }
    nats = {
      image       = "public.ecr.aws/docker/library/nats:2.11-alpine"
      entry_point = local.write["/tmp/nats.conf"]
      command     = ["nats-server", "--config", "/tmp/nats.conf"]
      secrets     = { NATS_CONF = "nats-conf" }
      memory      = var.nats_stream_mib + 64
    }
    caddy = {
      image       = "public.ecr.aws/docker/library/caddy:2-alpine"
      entry_point = local.write["/etc/caddy/Caddyfile"]
      command     = ["caddy", "run", "--config", "/etc/caddy/Caddyfile"]
      environment = { CADDYFILE = local.caddyfile }
      # Its certificates, which Let's Encrypt gives only so many of a week.
      volumes = { "/data/caddy" = "/data" }
      memory  = 32
    }
    api = {
      image       = module.cluster.image["api"]
      entry_point = local.write["aspen.toml"]
      command     = ["aspen-chat-server", "--no-https", "--listen-addr", "127.0.0.1", "--port", "8080"]
      environment = var.server_environment
      secrets     = { ASPEN_TOML = "aspen-toml" }
      memory      = 512
    }
    voice = {
      image       = module.cluster.image["voice"]
      entry_point = local.write["voice_server.toml"]
      command     = ["voice_server"]
      secrets     = { VOICE_SERVER_TOML = "voice-server-toml" }
      memory      = 256
    }
    # Before either server of a release starts: the tables it expects, and the voice server
    # under the id its NATS user and its own configuration name. The first release may find
    # PostgreSQL still making its database.
    release = {
      image       = module.cluster.image["api"]
      entry_point = local.write["aspen.toml"]
      command = ["sh", "-ec", <<-EOT
        tries=0
        until aspen-migrate up; do
          tries=$((tries + 1))
          [ "$tries" -lt 12 ] || exit 1
          sleep 5
        done
        aspen-chat-server voice-servers add ${local.voice_server_name} --url https://${local.voice_domain} --capacity ${var.voice_capacity} --id ${random_uuid.voice.result}
      EOT
      ]
      secrets = { ASPEN_TOML = "aspen-toml" }
      memory  = 128
    }
  }

  # Started with the machine, from images anyone can pull.
  services = toset(["postgres", "valkey", "nats", "caddy"])
  servers  = toset(["api", "voice"])
}

resource "aws_ecs_task_definition" "this" {
  for_each                 = local.containers
  family                   = "${var.name}-${each.key}"
  requires_compatibilities = ["EC2"]
  network_mode             = "host"
  execution_role_arn       = module.cluster.execution_role_arn

  dynamic "volume" {
    for_each = try(each.value.volumes, {})

    content {
      name      = basename(volume.key)
      host_path = volume.key
    }
  }

  container_definitions = jsonencode([merge(
    {
      name              = "aspen"
      image             = each.value.image
      essential         = true
      command           = each.value.command
      memoryReservation = each.value.memory
      environment       = [for name, value in try(each.value.environment, {}) : { name = name, value = value }]
      secrets           = [for name, key in try(each.value.secrets, {}) : { name = name, valueFrom = local.config_secret[key] }]
      mountPoints       = [for host, path in try(each.value.volumes, {}) : { sourceVolume = basename(host), containerPath = path }]
      ulimits           = [{ name = "nofile", softLimit = 65536, hardLimit = 65536 }]
      logConfiguration  = module.cluster.log_configuration[each.key]
    },
    can(each.value.entry_point) ? { entryPoint = each.value.entry_point } : {},
    can(each.value.stop_timeout) ? { stopTimeout = each.value.stop_timeout } : {},
  )])
}

resource "aws_ecs_service" "service" {
  for_each        = local.services
  name            = each.key
  cluster         = module.cluster.id
  task_definition = aws_ecs_task_definition.this[each.key].arn
  launch_type     = "EC2"
  desired_count   = 1

  # One of each on one machine's ports: the old one stops before the new one starts.
  deployment_minimum_healthy_percent = 0
  deployment_maximum_percent         = 100

  # An apply goes on only once each is running again, since the release, which follows, needs
  # the database and NATS.
  wait_for_steady_state = true

  depends_on = [aws_volume_attachment.data, aws_eip_association.this, aws_route_table_association.this]
}

# The order docs/operators/installing/upgrading.md gives: the release's work during the apply,
# then the servers. It runs again whenever its task definition changes, which a new image or a
# changed aspen.toml does; its commands change nothing when there is nothing to change.
resource "terraform_data" "release" {
  count            = local.deployed ? 1 : 0
  triggers_replace = [aws_ecs_task_definition.this["release"].arn]

  provisioner "local-exec" {
    command = "python3 ${path.module}/../aws/run_task.py"

    environment = {
      ASPEN_RUN_REGION          = var.region
      ASPEN_RUN_CLUSTER         = module.cluster.name
      ASPEN_RUN_TASK_DEFINITION = aws_ecs_task_definition.this["release"].arn
      ASPEN_RUN_LOG_GROUP       = module.cluster.log_group
      ASPEN_RUN_LOG_PREFIX      = "release"
    }
  }

  depends_on = [aws_ecs_service.service, aws_secretsmanager_secret_version.config]
}

resource "aws_ecs_service" "server" {
  for_each        = local.servers
  name            = each.key
  cluster         = module.cluster.id
  task_definition = aws_ecs_task_definition.this[each.key].arn
  launch_type     = "EC2"
  desired_count   = local.deployed ? 1 : 0

  # The deployment is down while its one API server is replaced, and calls end while the voice
  # server is. Apps reconnect and rejoin on their own.
  deployment_minimum_healthy_percent = 0
  deployment_maximum_percent         = 100

  depends_on = [terraform_data.release]
}
