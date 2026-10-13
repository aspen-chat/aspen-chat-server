# Plans the configuration against providers that answer with made-up values, so it needs no AWS
# account: `tofu test`. It checks what a mistake here would open or break without any error,
# not that AWS accepts the result.

# The provider checks the shape of an ARN wherever one is given, and the certificate's record
# is read from its validation options, so those the mocks must answer in kind.
mock_provider "aws" {
  mock_resource "aws_iam_role" {
    defaults = { arn = "arn:aws:iam::123456789012:role/mock" }
  }
  mock_resource "aws_ebs_volume" {
    defaults = { id = "vol-0123456789abcdef0" }
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

variables {
  region          = "eu-central-1"
  domain          = "chat.example.org"
  route53_zone_id = "Z0000000000000"
}

run "infrastructure_only" {
  command = plan

  assert {
    condition     = alltrue([for server in aws_ecs_service.server : server.desired_count == 0]) && length(terraform_data.release) == 0
    error_message = "Without an image tag neither server is started and no release is run: there is no image to pull."
  }

  assert {
    condition     = alltrue([for service in aws_ecs_service.service : service.desired_count == 1])
    error_message = "The services run from the first apply."
  }
}

run "release" {
  command = plan

  variables {
    image_tag = "2026-10-12"
  }

  assert {
    condition     = alltrue([for server in aws_ecs_service.server : server.desired_count == 1]) && length(terraform_data.release) == 1
    error_message = "A release runs the release task and both servers."
  }

  assert {
    condition = toset([
      for rule in aws_vpc_security_group_ingress_rule.public : "${rule.description}:${rule.ip_protocol}:${rule.from_port}-${rule.to_port}"
      ]) == toset([
      "http:tcp:80-80",
      "https:tcp:443-443",
      "voice-media-udp:udp:40000-40199",
      "voice-media-tcp:tcp:40000-40199",
      "voice-file-transfers:udp:3478-3478",
    ])
    error_message = "Only Caddy's ports and the voice server's media and file transfer ports are open to the internet (docs/operators/installing/network-exposure.md)."
  }

  # Every container is on the machine's network, so a service that listened on every interface
  # would be one security group rule from the internet.
  assert {
    condition = (
      contains(local.containers.postgres.command, "listen_addresses=127.0.0.1")
      && contains(local.containers.valkey.command, "127.0.0.1")
      && contains(local.containers.api.command, "127.0.0.1")
      && contains(local.containers.valkey.command, "noeviction")
    )
    error_message = "PostgreSQL, Valkey, and the API server listen on loopback, and Valkey never evicts."
  }

  assert {
    condition     = local.trusted_proxies == ["127.0.0.1"]
    error_message = "Both servers believe Caddy, on this machine, about client addresses."
  }

  assert {
    condition     = aws_instance.this.instance_type == "t3a.medium"
    error_message = "The x86 default is t3a.medium."
  }

  assert {
    condition     = local.data_device == "/dev/disk/by-id/nvme-Amazon_Elastic_Block_Store_vol0123456789abcdef0"
    error_message = "The data volume is found by its id, as NVMe names it."
  }
}

run "export" {
  command = plan

  assert {
    condition     = contains(local.dump_command, "--host=127.0.0.1") && contains(local.dump_command, "--format=custom")
    error_message = "The export dumps this machine's database in the format the restore reads."
  }

  assert {
    condition     = alltrue([for address in module.media.handed_over : !strcontains(address, "iam")])
    error_message = "A deployment taking the storage over makes a key pair of its own; this one's is destroyed with it."
  }
}

run "arm" {
  command = plan

  variables {
    cpu_architecture = "ARM64"
  }

  assert {
    condition     = aws_instance.this.instance_type == "t4g.medium" && data.aws_ssm_parameter.ecs_ami.name == "/aws/service/ecs/optimized-ami/amazon-linux-2023/arm64/recommended/image_id"
    error_message = "An ARM deployment runs a Graviton machine from the ARM image."
  }
}

run "refuses_too_little_memory_for_nats" {
  command = plan

  variables {
    nats_stream_mib = 64
  }

  expect_failures = [var.nats_stream_mib]
}
