# The three configuration files, written as the operator guide describes them and handed to
# each container as a secret it writes into its working directory before starting.

resource "random_password" "nats_server" {
  length  = 40
  special = false
}

resource "random_password" "nats_voice" {
  for_each = var.voice_servers
  length   = 40
  special  = false
}

# The id each voice server is registered under and reports as, which its NATS user's
# permissions name too. A server removed from `voice_servers` and added again gets another.
resource "random_uuid" "voice" {
  for_each = var.voice_servers
}

locals {
  public_url       = "https://${var.domain}"
  nats_host        = "${aws_service_discovery_service.nats.name}.${aws_service_discovery_private_dns_namespace.this.name}"
  nats_url         = "nats://${local.nats_host}:4222"
  nats_monitor_url = "http://${local.nats_host}:8222"

  # The load balancer's nodes take addresses in the public subnets, and are the only proxies
  # the security groups let reach either server.
  trusted_proxies = local.public_cidrs

  aspen_toml = templatefile("${path.module}/templates/aspen.toml.tftpl", {
    public_url       = local.public_url
    database_url     = local.database_url
    valkey_url       = local.valkey_url
    nats_url         = local.nats_url
    nats_password    = random_password.nats_server.result
    nats_monitor_url = local.nats_monitor_url
    trusted_proxies  = local.trusted_proxies
    s3_endpoint      = "https://s3.${var.region}.amazonaws.com"
    media_url        = module.media.public_base_url
    bucket           = module.media.bucket
    region           = var.region
    s3_access_key    = module.media.access_key
    s3_secret_key    = module.media.secret_key
    email            = var.email
  })

  voice_server_toml = {
    for name, voice in var.voice_servers : name => templatefile("${path.module}/templates/voice_server.toml.tftpl", {
      id              = random_uuid.voice[name].result
      name            = name
      nats_url        = local.nats_url
      listen_addr     = "0.0.0.0:9000"
      nats_password   = random_password.nats_voice[name].result
      trusted_proxies = local.trusted_proxies
      public_address  = aws_eip.voice[name].public_ip
      min_port        = var.voice_media_ports.min
      max_port        = var.voice_media_ports.max
    })
  }

  nats_conf = templatefile("${path.module}/templates/nats.conf.tftpl", {
    # NATS reads the machine's memory, not its container's, so it is told what it may reserve.
    max_memory_store_bytes = floor(var.nats.memory * 5 / 8) * 1024 * 1024
    listen_address         = "0.0.0.0"
    server_password        = random_password.nats_server.result
    voice_servers = {
      for name, _ in var.voice_servers : name => {
        id       = random_uuid.voice[name].result
        password = random_password.nats_voice[name].result
      }
    }
  })

  configs = merge(
    { aspen-toml = local.aspen_toml, nats-conf = local.nats_conf },
    { for name, toml in local.voice_server_toml : "voice-${name}" => toml },
  )
}

resource "aws_secretsmanager_secret" "config" {
  for_each    = toset(concat(["aspen-toml", "nats-conf"], [for name in keys(var.voice_servers) : "voice-${name}"]))
  name_prefix = "${var.name}/${each.key}-"
}

resource "aws_secretsmanager_secret_version" "config" {
  for_each      = aws_secretsmanager_secret.config
  secret_id     = each.value.id
  secret_string = local.configs[each.key]
}

locals {
  # Each names one version of its file, so a changed file is a changed task definition, which
  # its service rolls out, and a task started later reads the file it was deployed with.
  config_secret = {
    for key, version in aws_secretsmanager_secret_version.config :
    key => "${version.secret_arn}:::${version.version_id}"
  }
}
