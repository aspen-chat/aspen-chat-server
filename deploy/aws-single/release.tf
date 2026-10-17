# Each release, done during the apply: `aspen-release` on the machine makes the box the first
# time and upgrades it after (machine/aspen-release). It runs again when the release changes or
# the machine is replaced, which only starts the box again.

locals {
  images = { for target, repository in var.image_repositories : target => "${repository}:${var.image_tag}" }

  # What `aspen-box init` is told, the first time alone: the box keeps its answers in box.json,
  # which is the deployment's configuration from then on.
  init_options = concat(
    [
      "--domain", var.domain,
      "--address4", aws_eip.this.public_ip,
      "--address6", local.ipv6_address,
      "--storage", "s3", "--s3-role",
      "--s3-endpoint", "https://s3.${var.region}.amazonaws.com",
      # Clients upload over IPv6 too: the dual-stack name, path-style as the server signs.
      "--s3-public-endpoint", "https://s3.dualstack.${var.region}.amazonaws.com",
      "--s3-bucket", module.media.bucket,
      "--s3-region", var.region,
      "--s3-public-base-url", module.media.public_base_url,
    ],
    var.acme_email == null ? [] : ["--acme-email", var.acme_email],
  )
}

resource "terraform_data" "release" {
  triggers_replace = [aws_instance.this.id, local.images]

  provisioner "local-exec" {
    command = join(" ", concat(
      ["python3", "${path.module}/on_machine.py", "--"],
      [for argument in concat(["aspen-release", local.images.api, local.images.voice], local.init_options) : "'${argument}'"],
    ))

    environment = {
      ASPEN_MACHINE_REGION   = var.region
      ASPEN_MACHINE_INSTANCE = aws_instance.this.id
    }
  }

  depends_on = [
    aws_volume_attachment.data,
    aws_eip_association.this,
    aws_route_table_association.this,
    aws_vpc_security_group_egress_rule.all,
    aws_iam_role_policy_attachment.machine,
    aws_iam_role_policy.media,
    aws_route53_record.machine,
  ]
}
