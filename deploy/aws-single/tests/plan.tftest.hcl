# Plans the configuration against providers that answer with made-up values, so it needs no AWS
# account: `tofu test`. It checks what a mistake here would open or break without any error,
# not that AWS accepts the result.

# The provider checks the shape of an ARN wherever one is given, and the certificate's record
# is read from its validation options, so those the mocks must answer in kind.
mock_provider "aws" {
  mock_resource "aws_vpc" {
    defaults = { ipv6_cidr_block = "2001:db8:42:ab00::/56" }
  }
  mock_resource "aws_iam_role" {
    defaults = { arn = "arn:aws:iam::123456789012:role/mock" }
  }
  mock_resource "aws_ebs_volume" {
    defaults = { id = "vol-0123456789abcdef0" }
  }
  mock_resource "aws_eip" {
    defaults = { public_ip = "203.0.113.7" }
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
  image_tag       = "2026-10-16"
}

run "release" {
  command = plan

  assert {
    condition = toset([
      for rule in aws_vpc_security_group_ingress_rule.public : "${rule.ip_protocol}:${rule.from_port}-${rule.to_port}:${coalesce(rule.cidr_ipv4, rule.cidr_ipv6)}"
      ]) == toset(flatten([for cidr in ["0.0.0.0/0", "::/0"] : [
        "tcp:80-80:${cidr}",
        "tcp:443-443:${cidr}",
        "udp:40000-40199:${cidr}",
        "tcp:40000-40199:${cidr}",
        "udp:3478-3478:${cidr}",
    ]]))
    error_message = "Only Caddy's ports and the voice server's media and file transfer ports are open to the internet, over both families (docs/operators/installing/network-exposure.md)."
  }

  assert {
    condition = (
      toset(keys(aws_route53_record.machine)) == toset(["chat.example.org A", "chat.example.org AAAA", "voice.chat.example.org A", "voice.chat.example.org AAAA"])
      && aws_route53_record.machine["voice.chat.example.org AAAA"].records == toset(["2001:db8:42:ab00::1000"])
      && aws_instance.this.ipv6_addresses == tolist(["2001:db8:42:ab00::1000"])
    )
    error_message = "Both names point at the machine's two addresses, which it is given."
  }

  assert {
    condition = (
      contains(local.init_options, "--s3-role") && !anytrue([for option in local.init_options : strcontains(option, "key")])
      && contains(local.init_options, "https://s3.dualstack.eu-central-1.amazonaws.com")
      && contains(local.init_options, "203.0.113.7") && contains(local.init_options, "2001:db8:42:ab00::1000")
    )
    error_message = "The box reaches the bucket as the machine's role, and announces both addresses: ${jsonencode(local.init_options)}"
  }

  assert {
    condition     = local.images == { api = "ghcr.io/aspen-chat/aspen-api:2026-10-16", voice = "ghcr.io/aspen-chat/aspen-voice:2026-10-16" }
    error_message = "The box runs the published images of the release."
  }

  assert {
    condition     = aws_instance.this.metadata_options[0].http_tokens == "required"
    error_message = "The machine's metadata, and its role's credentials with it, need a token."
  }

  assert {
    condition     = strcontains(aws_instance.this.user_data, "[systemctl, enable, --now, aspen-voice-imds.service]")
    error_message = "The machine refuses the voice server its metadata, and its role's credentials with it."
  }

  # Anything in user_data that a variable decides would replace the machine when it changed.
  assert {
    condition     = !strcontains(aws_instance.this.user_data, "2026-10-16") && strcontains(aws_instance.this.user_data, local.data_device)
    error_message = "The machine's setup names its data volume and nothing of the release."
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

run "handover" {
  command = plan

  assert {
    condition     = contains(output.handover.kept, "aws_ebs_volume.data") && contains(output.handover.kept, "aws_route53_record.machine[\"chat.example.org AAAA\"]")
    error_message = "The larger deployment keeps the data volume as the way back, and takes the name over."
  }
}

run "arm" {
  command = plan

  variables {
    cpu_architecture = "ARM64"
  }

  assert {
    condition     = aws_instance.this.instance_type == "t4g.medium" && strcontains(data.aws_ssm_parameter.ubuntu.name, "/arm64/")
    error_message = "An ARM deployment runs a Graviton machine from the ARM image."
  }
}
