# The configuration files, written from the templates the larger configuration (../aws) uses,
# with every service at this machine's loopback address.

resource "random_password" "this" {
  for_each = toset(["database", "nats-server", "nats-voice"])
  length   = 40
  # Letters and digits alone, so none needs escaping where it is written.
  special = false
}

# The id the voice server is registered under and reports as, which its NATS user's
# permissions name too.
resource "random_uuid" "voice" {}

module "media" {
  source = "../aws/modules/media"

  name            = var.name
  domain          = "media.${var.domain}"
  route53_zone_id = var.route53_zone_id

  providers = {
    aws           = aws
    aws.us_east_1 = aws.us_east_1
  }
}

locals {
  public_url = "https://${var.domain}"
  # Caddy, the only thing that reaches either server.
  trusted_proxies = ["127.0.0.1"]

  configs = {
    aspen-toml = templatefile("${path.module}/../aws/templates/aspen.toml.tftpl", {
      public_url       = local.public_url
      database_url     = "postgres://aspen:${random_password.this["database"].result}@127.0.0.1:5432/aspen"
      valkey_url       = "redis://127.0.0.1:6379"
      nats_url         = "nats://127.0.0.1:4222"
      nats_password    = random_password.this["nats-server"].result
      nats_monitor_url = "http://127.0.0.1:8222"
      trusted_proxies  = local.trusted_proxies
      s3_endpoint      = "https://s3.${var.region}.amazonaws.com"
      media_url        = module.media.public_base_url
      bucket           = module.media.bucket
      region           = var.region
      s3_access_key    = module.media.access_key
      s3_secret_key    = module.media.secret_key
      email            = var.email
    })

    voice-server-toml = templatefile("${path.module}/../aws/templates/voice_server.toml.tftpl", {
      id              = random_uuid.voice.result
      name            = local.voice_server_name
      nats_url        = "nats://127.0.0.1:4222"
      listen_addr     = "127.0.0.1:9000"
      nats_password   = random_password.this["nats-voice"].result
      trusted_proxies = local.trusted_proxies
      public_address  = aws_eip.this.public_ip
      min_port        = var.voice_media_ports.min
      max_port        = var.voice_media_ports.max
    })

    nats-conf = templatefile("${path.module}/../aws/templates/nats.conf.tftpl", {
      listen_address         = "127.0.0.1"
      max_memory_store_bytes = var.nats_stream_mib * 1024 * 1024
      server_password        = random_password.this["nats-server"].result
      voice_servers = {
        (local.voice_server_name) = { id = random_uuid.voice.result, password = random_password.this["nats-voice"].result }
      }
    })

    database-password = random_password.this["database"].result
  }

  caddyfile = templatefile("${path.module}/templates/Caddyfile.tftpl", {
    acme_email   = var.acme_email
    domain       = var.domain
    voice_domain = local.voice_domain
  })
}

resource "aws_secretsmanager_secret" "config" {
  for_each    = toset(["aspen-toml", "voice-server-toml", "nats-conf", "database-password"])
  name_prefix = "${var.name}/${each.key}-"
}

resource "aws_secretsmanager_secret_version" "config" {
  for_each      = aws_secretsmanager_secret.config
  secret_id     = each.value.id
  secret_string = local.configs[each.key]
}

locals {
  # Each names one version of its file, so a changed file is a changed task definition, which
  # its service rolls out.
  config_secret = {
    for key, version in aws_secretsmanager_secret_version.config :
    key => "${version.secret_arn}:::${version.version_id}"
  }
}
