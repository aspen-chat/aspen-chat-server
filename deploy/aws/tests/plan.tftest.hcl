# Plans the configuration against providers that answer with made-up values, so it needs no AWS
# account: `tofu test`. It checks what a mistake here would open or break without any error,
# not that AWS accepts the result.

# The provider checks the shape of an ARN wherever one is given, and the certificates' records
# are read by name, so those the mocks must answer in kind.
mock_provider "aws" {
  mock_resource "aws_iam_role" {
    defaults = { arn = "arn:aws:iam::123456789012:role/mock" }
  }
  mock_resource "aws_service_discovery_service" {
    defaults = { arn = "arn:aws:servicediscovery:eu-central-1:123456789012:service/srv-mock" }
  }
  mock_resource "aws_lb" {
    defaults = { arn = "arn:aws:elasticloadbalancing:eu-central-1:123456789012:loadbalancer/app/mock/0123456789abcdef" }
  }
  mock_resource "aws_lb_listener" {
    defaults = { arn = "arn:aws:elasticloadbalancing:eu-central-1:123456789012:listener/app/mock/0123456789abcdef/0123456789abcdef" }
  }
  mock_resource "aws_lb_target_group" {
    defaults = { arn = "arn:aws:elasticloadbalancing:eu-central-1:123456789012:targetgroup/mock/0123456789abcdef" }
  }
  mock_resource "aws_acm_certificate" {
    defaults = {
      arn = "arn:aws:acm:eu-central-1:123456789012:certificate/mock"
      domain_validation_options = [{
        domain_name           = "chat.example.org"
        resource_record_name  = "_mock.chat.example.org."
        resource_record_type  = "CNAME"
        resource_record_value = "_mock.acm-validations.aws."
      }]
    }
  }
}

mock_provider "aws" {
  alias = "us_east_1"

  mock_resource "aws_acm_certificate" {
    defaults = {
      arn = "arn:aws:acm:us-east-1:123456789012:certificate/mock"
      domain_validation_options = [{
        domain_name           = "media.chat.example.org"
        resource_record_name  = "_mock.media.chat.example.org."
        resource_record_type  = "CNAME"
        resource_record_value = "_mock.acm-validations.aws."
      }]
    }
  }
}

override_data {
  target = data.http.rds_ca
  values = {
    status_code   = 200
    response_body = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n"
  }
}

variables {
  region          = "eu-central-1"
  domain          = "chat.example.org"
  route53_zone_id = "Z0000000000000"
  voice_servers = {
    voice-1 = { capacity = 120 }
    voice-2 = { capacity = 60, instance_type = "c7i.xlarge" }
  }
}

run "infrastructure_only" {
  command = plan

  assert {
    condition     = aws_ecs_service.api.desired_count == 0 && alltrue([for service in aws_ecs_service.voice : service.desired_count == 0])
    error_message = "Without an image tag no server may be started: there is no image to pull."
  }

  assert {
    condition     = length(terraform_data.release) == 0
    error_message = "Without an image tag there is no release to run."
  }
}

run "release" {
  command = plan

  variables {
    image_tag = "2026-10-12"
  }

  assert {
    condition     = aws_ecs_service.api.desired_count == 2 && length(terraform_data.release) == 1
    error_message = "A release runs the release task and the API servers."
  }

  assert {
    condition     = local.trusted_proxies == ["10.42.0.0/20", "10.42.16.0/20"]
    error_message = "Both servers must believe the load balancer's subnets, and nothing else, about client addresses."
  }

  # The private subnets must not overlap the ones the servers trust as proxies.
  assert {
    condition     = local.private_cidrs == ["10.42.128.0/20", "10.42.144.0/20"]
    error_message = "The private subnets moved."
  }

  assert {
    condition = toset([
      for rule in aws_vpc_security_group_ingress_rule.public : "${rule.description}:${rule.ip_protocol}:${rule.from_port}-${rule.to_port}"
      ]) == toset([
      "load-balancer-http:tcp:80-80",
      "load-balancer-https:tcp:443-443",
      "voice-media-udp:udp:40000-40999",
      "voice-media-tcp:tcp:40000-40999",
      "voice-file-transfers:udp:3478-3478",
    ])
    error_message = "Only the load balancer's ports and the voice servers' media and file transfer ports are open to the internet (docs/operators/installing/network-exposure.md)."
  }

  assert {
    condition     = !aws_db_instance.this.publicly_accessible && aws_elasticache_replication_group.this.transit_encryption_enabled
    error_message = "The database is private, and Valkey is reached over TLS."
  }

  assert {
    condition     = local.public_prefixes == ["attachments", "attachment-previews", "icons", "link-preview-images"]
    error_message = "The read path serves the four prefixes clients read (docs/operators/installing/storage-read-path.md)."
  }

  assert {
    condition = alltrue([
      for parameter in aws_elasticache_parameter_group.this.parameter : parameter.value == "noeviction"
      if parameter.name == "maxmemory-policy"
    ]) && length(aws_elasticache_parameter_group.this.parameter) == 1
    error_message = "Valkey must never evict."
  }

  assert {
    condition     = aws_instance.voice["voice-1"].instance_type == "c7i.large" && aws_instance.voice["voice-2"].instance_type == "c7i.xlarge"
    error_message = "A voice server takes its own instance type, or the architecture's default."
  }

}

run "arm" {
  command = plan

  variables {
    image_tag        = "2026-10-12"
    cpu_architecture = "ARM64"
  }

  assert {
    condition     = local.ecs_ami_parameter == "/aws/service/ecs/optimized-ami/amazon-linux-2023/arm64/recommended/image_id"
    error_message = "ARM voice servers boot the ARM image."
  }

  assert {
    condition     = aws_instance.voice["voice-1"].instance_type == "c7g.large"
    error_message = "ARM voice servers default to a Graviton instance."
  }
}

run "refuses_a_voice_server_named_media" {
  command = plan

  variables {
    voice_servers = { media = { capacity = 10 } }
  }

  expect_failures = [var.voice_servers]
}
