# The one machine everything runs on, its address, and the volume that outlives it.

locals {
  zone          = "${var.region}${var.availability_zone_suffix}"
  arm           = var.cpu_architecture == "ARM64"
  instance_type = coalesce(var.instance_type, local.arm ? "t4g.medium" : "t3a.medium")
  voice_domain  = "voice.${var.domain}"

  # Where the volume appears on the machine, whatever order its disks are found in.
  data_device = "/dev/disk/by-id/nvme-Amazon_Elastic_Block_Store_${replace(aws_ebs_volume.data.id, "-", "")}"
}

resource "aws_vpc" "this" {
  cidr_block           = "10.42.0.0/24"
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = { Name = var.name }
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = var.name }
}

resource "aws_subnet" "this" {
  vpc_id                  = aws_vpc.this.id
  availability_zone       = local.zone
  cidr_block              = aws_vpc.this.cidr_block
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
}

resource "aws_route_table_association" "this" {
  subnet_id      = aws_subnet.this.id
  route_table_id = aws_route_table.this.id
}

# The only firewall: every container is on the machine's own network. The database, NATS,
# Valkey, and both servers listen on loopback, behind Caddy, so none of their ports is here
# (docs/operators/installing/network-exposure.md).
resource "aws_security_group" "this" {
  name        = var.name
  description = "Aspen ${var.name}"
  vpc_id      = aws_vpc.this.id
  tags        = { Name = var.name }
}

locals {
  # Protocol and ports open to everyone.
  public_rules = {
    http                 = ["tcp", 80, 80]
    https                = ["tcp", 443, 443]
    voice-media-udp      = ["udp", var.voice_media_ports.min, var.voice_media_ports.max]
    voice-media-tcp      = ["tcp", var.voice_media_ports.min, var.voice_media_ports.max]
    voice-file-transfers = ["udp", 3478, 3478]
  }
}

resource "aws_vpc_security_group_ingress_rule" "public" {
  for_each          = local.public_rules
  security_group_id = aws_security_group.this.id
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = each.value[0]
  from_port         = each.value[1]
  to_port           = each.value[2]
  description       = each.key
}

resource "aws_vpc_security_group_egress_rule" "all" {
  security_group_id = aws_security_group.this.id
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = "-1"
}

# What the ECS agent needs to join the cluster and pull images, and what Session Manager needs
# to open a shell: no SSH port is open. It cannot read the configuration files' secrets.
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
  for_each = toset([
    "arn:aws:iam::aws:policy/service-role/AmazonEC2ContainerServiceforEC2Role",
    "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore",
  ])
  role       = aws_iam_role.machine.name
  policy_arn = each.value
}

resource "aws_iam_instance_profile" "machine" {
  name = "${var.name}-machine"
  role = aws_iam_role.machine.name
}

# The database's files and Caddy's certificates. Daily snapshots of it are the deployment's
# backups: one taken while PostgreSQL runs restores as a database that lost power does.
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

data "aws_ssm_parameter" "ecs_ami" {
  name = "/aws/service/ecs/optimized-ami/amazon-linux-2023/${local.arm ? "arm64/" : ""}recommended/image_id"
}

# The address the deployment's names point at and clients send media to, kept when the machine
# is replaced.
resource "aws_eip" "this" {
  domain = "vpc"
  tags   = { Name = var.name }
}

resource "aws_instance" "this" {
  ami                    = data.aws_ssm_parameter.ecs_ami.value
  instance_type          = local.instance_type
  subnet_id              = aws_subnet.this.id
  vpc_security_group_ids = [aws_security_group.this.id]
  iam_instance_profile   = aws_iam_instance_profile.machine.name
  tags                   = { Name = var.name }

  # Runs once, before the ECS agent starts, which waits for it: no container starts until the
  # data volume is mounted, so PostgreSQL never makes an empty database on the machine's own
  # disk. `mkfs.xfs` refuses a volume that already holds a filesystem. On later boots the
  # mount is /etc/fstab's, and a machine whose volume is missing does not finish booting.
  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    device=${local.data_device}
    for attempt in $(seq 1 120); do
      [ -e "$device" ] && break
      sleep 5
    done
    blkid "$device" > /dev/null || mkfs.xfs "$device"
    mkdir -p /data
    echo "UUID=$(blkid -s UUID -o value "$device") /data xfs defaults 0 2" >> /etc/fstab
    mount /data
    mkdir -p /data/postgres /data/caddy
    echo "ECS_CLUSTER=${module.cluster.name}" >> /etc/ecs/ecs.config
  EOT

  metadata_options {
    http_tokens = "required"
  }

  root_block_device {
    encrypted = true
  }

  lifecycle {
    # A newer recommended image would otherwise replace the machine, and stop the deployment
    # for some minutes, on whichever apply came next. To move to the current image:
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
  for_each = toset([var.domain, local.voice_domain])
  zone_id  = var.route53_zone_id
  name     = each.value
  type     = "A"
  ttl      = 300
  records  = [aws_eip.this.public_ip]
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
