# The one machine the box runs on, its addresses, and the volume that outlives it.

locals {
  zone          = "${var.region}${var.availability_zone_suffix}"
  arm           = var.cpu_architecture == "ARM64"
  instance_type = coalesce(var.instance_type, local.arm ? "t4g.medium" : "t3a.medium")
  voice_domain  = "voice.${var.domain}"

  # Where the volume appears on the machine, whatever order its disks are found in.
  data_device = "/dev/disk/by-id/nvme-Amazon_Elastic_Block_Store_${replace(aws_ebs_volume.data.id, "-", "")}"

  # The machine's IPv6 address, chosen rather than given so that it stays the same when the
  # machine is replaced, as its Elastic IP does.
  ipv6_address = cidrhost(aws_subnet.this.ipv6_cidr_block, 4096)

  # The media ports `aspen-box init` gives the voice server (`rtc` in box.json).
  media_ports = { min = 40000, max = 40199 }
}

resource "aws_vpc" "this" {
  cidr_block                       = "10.42.0.0/24"
  enable_dns_support               = true
  enable_dns_hostnames             = true
  assign_generated_ipv6_cidr_block = true
  tags                             = { Name = var.name }
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = var.name }
}

resource "aws_subnet" "this" {
  vpc_id                  = aws_vpc.this.id
  availability_zone       = local.zone
  cidr_block              = aws_vpc.this.cidr_block
  ipv6_cidr_block         = cidrsubnet(aws_vpc.this.ipv6_cidr_block, 8, 0)
  map_public_ip_on_launch = true
  tags                    = { Name = var.name }
}

resource "aws_route_table" "this" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = var.name }

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.this.id
  }

  route {
    ipv6_cidr_block = "::/0"
    gateway_id      = aws_internet_gateway.this.id
  }
}

resource "aws_route_table_association" "this" {
  subnet_id      = aws_subnet.this.id
  route_table_id = aws_route_table.this.id
}

# The only firewall. The box's services listen on loopback behind Caddy, so none of their ports
# is here (docs/operators/installing/network-exposure.md).
resource "aws_security_group" "this" {
  name        = var.name
  description = "Aspen ${var.name}"
  vpc_id      = aws_vpc.this.id
  tags        = { Name = var.name }
}

locals {
  # Protocol and ports open to everyone, over IPv4 and IPv6.
  public_ports = {
    http                 = ["tcp", 80, 80]
    https                = ["tcp", 443, 443]
    voice-media-udp      = ["udp", local.media_ports.min, local.media_ports.max]
    voice-media-tcp      = ["tcp", local.media_ports.min, local.media_ports.max]
    voice-file-transfers = ["udp", 3478, 3478]
  }
  everyone = { ipv4 = "0.0.0.0/0", ipv6 = "::/0" }
}

resource "aws_vpc_security_group_ingress_rule" "public" {
  for_each = merge([
    for family, cidr in local.everyone : { for name, rule in local.public_ports : "${name}-${family}" => concat(rule, [family, cidr]) }
  ]...)
  security_group_id = aws_security_group.this.id
  cidr_ipv4         = each.value[3] == "ipv4" ? each.value[4] : null
  cidr_ipv6         = each.value[3] == "ipv6" ? each.value[4] : null
  ip_protocol       = each.value[0]
  from_port         = each.value[1]
  to_port           = each.value[2]
  description       = each.key
}

resource "aws_vpc_security_group_egress_rule" "all" {
  for_each          = local.everyone
  security_group_id = aws_security_group.this.id
  cidr_ipv4         = each.key == "ipv4" ? each.value : null
  cidr_ipv6         = each.key == "ipv6" ? each.value : null
  ip_protocol       = "-1"
}

# What Session Manager needs to run commands and open a shell (no SSH port is open), and what the
# API server may do with the bucket, as the machine's role.
resource "aws_iam_role" "machine" {
  name = "${var.name}-machine"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "machine" {
  role       = aws_iam_role.machine.name
  policy_arn = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore"
}

resource "aws_iam_role_policy" "media" {
  name   = "media"
  role   = aws_iam_role.machine.id
  policy = module.media.access_policy
}

resource "aws_iam_instance_profile" "machine" {
  name = "${var.name}-machine"
  role = aws_iam_role.machine.name
}

# The box: its box.json and the passwords it made, the database, the certificates, and its
# backups. Daily snapshots of it are the deployment's backups: one taken while PostgreSQL runs
# restores as a database that lost power does.
resource "aws_ebs_volume" "data" {
  availability_zone = local.zone
  size              = var.data_volume_gb
  type              = "gp3"
  encrypted         = true
  tags              = { Name = "${var.name}-data", Backup = var.name }

  lifecycle {
    prevent_destroy = true
  }
}

data "aws_ssm_parameter" "ubuntu" {
  name = "/aws/service/canonical/ubuntu/server/24.04/stable/current/${local.arm ? "arm64" : "amd64"}/hvm/ebs-gp3/ami-id"
}

# The address the deployment's names point at and clients send media to, kept when the machine
# is replaced.
resource "aws_eip" "this" {
  domain = "vpc"
  tags   = { Name = var.name }
}

resource "aws_instance" "this" {
  ami                    = data.aws_ssm_parameter.ubuntu.value
  instance_type          = local.instance_type
  subnet_id              = aws_subnet.this.id
  ipv6_addresses         = [local.ipv6_address]
  vpc_security_group_ids = [aws_security_group.this.id]
  iam_instance_profile   = aws_iam_instance_profile.machine.name
  tags                   = { Name = var.name }

  # Docker and the scripts in machine/, the voice server refused the instance metadata service
  # (machine/voice-imds.service), then the volume mounted. Nothing of the box starts
  # here: the release does that (release.tf), once this has finished. Ubuntu's own Docker
  # comes with Compose 2.20 or later, which the box needs.
  user_data = <<-EOT
    #cloud-config
    package_update: true
    packages: [docker.io, docker-compose-v2]
    write_files:
      - path: /usr/local/sbin/aspen-setup
        permissions: "0755"
        encoding: b64
        content: ${filebase64("${path.module}/machine/setup")}
      - path: /usr/local/sbin/aspen-release
        permissions: "0755"
        encoding: b64
        content: ${filebase64("${path.module}/machine/aspen-release")}
      - path: /usr/local/bin/aspen-box
        permissions: "0755"
        encoding: b64
        content: ${filebase64("${path.module}/machine/aspen-box")}
      - path: /etc/systemd/system/docker.service.d/data.conf
        encoding: b64
        content: ${filebase64("${path.module}/machine/docker-data.conf")}
      - path: /etc/systemd/system/aspen-voice-imds.service
        encoding: b64
        content: ${filebase64("${path.module}/machine/voice-imds.service")}
    runcmd:
      - [systemctl, enable, --now, aspen-voice-imds.service]
      - [/usr/local/sbin/aspen-setup, "${local.data_device}"]
  EOT

  metadata_options {
    http_tokens = "required"
  }

  root_block_device {
    encrypted = true
  }

  lifecycle {
    # A newer Ubuntu image would otherwise replace the machine, and stop the deployment for some
    # minutes, on whichever apply came next. To move to the current image:
    # tofu apply -replace=aws_instance.this
    ignore_changes = [ami]
  }
}

resource "aws_volume_attachment" "data" {
  device_name = "/dev/sdf"
  volume_id   = aws_ebs_volume.data.id
  instance_id = aws_instance.this.id
}

resource "aws_eip_association" "this" {
  instance_id   = aws_instance.this.id
  allocation_id = aws_eip.this.id
}

resource "aws_route53_record" "machine" {
  for_each = { for pair in setproduct([var.domain, local.voice_domain], ["A", "AAAA"]) : "${pair[0]} ${pair[1]}" => pair }
  zone_id  = var.route53_zone_id
  name     = each.value[0]
  type     = each.value[1]
  ttl      = 300
  records  = [each.value[1] == "A" ? aws_eip.this.public_ip : local.ipv6_address]
}

resource "aws_iam_role" "backup" {
  name = "${var.name}-backup"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "dlm.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "backup" {
  role       = aws_iam_role.backup.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSDataLifecycleManagerServiceRole"
}

resource "aws_dlm_lifecycle_policy" "backup" {
  description        = "Aspen ${var.name} daily snapshots of the data volume"
  execution_role_arn = aws_iam_role.backup.arn
  state              = "ENABLED"

  policy_details {
    resource_types = ["VOLUME"]
    target_tags    = { Backup = var.name }

    schedule {
      name      = "daily"
      copy_tags = true

      create_rule {
        interval      = 24
        interval_unit = "HOURS"
        times         = ["04:00"]
      }

      retain_rule {
        count = var.backup_retention_days
      }
    }
  }
}
