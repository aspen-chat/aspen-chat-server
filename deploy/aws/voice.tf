# The voice servers, each a machine of its own with an address of its own: people's devices
# send media straight to it, over a range of UDP ports no load balancer carries. Signalling
# alone goes through the load balancer, which holds the certificate for its name.

locals {
  voice_instance_type = var.cpu_architecture == "ARM64" ? "c7g.large" : "c7i.large"
  ecs_ami_parameter   = "/aws/service/ecs/optimized-ami/amazon-linux-2023/${var.cpu_architecture == "ARM64" ? "arm64/" : ""}recommended/image_id"
}

data "aws_ssm_parameter" "ecs_ami" {
  name = local.ecs_ami_parameter
}

# What the ECS agent on each machine needs to join the cluster and pull the image, and what
# Session Manager needs to open a shell there: no SSH port is open.
resource "aws_iam_role" "voice_instance" {
  name = "${var.name}-voice-instance"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "voice_instance" {
  for_each = toset([
    "arn:aws:iam::aws:policy/service-role/AmazonEC2ContainerServiceforEC2Role",
    "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore",
  ])
  role       = aws_iam_role.voice_instance.name
  policy_arn = each.value
}

resource "aws_iam_instance_profile" "voice" {
  name = "${var.name}-voice"
  role = aws_iam_role.voice_instance.name
}

# The address clients send media to (`[rtc] announced_address`), kept when the machine is
# replaced.
resource "aws_eip" "voice" {
  for_each = var.voice_servers
  domain   = "vpc"
  tags     = { Name = "${var.name}-${each.key}" }
}

locals {
  # Each voice server's subnet, and its IPv6 address there, which clients send media to over
  # IPv6 (`[rtc] announced_address`). Both follow from its name alone, so the address stays the
  # same when the machine is replaced, as its Elastic IP does, and adding or removing another
  # server moves none: a number from the name's hash, at least 2^32 into the subnet, far from
  # the addresses AWS hands out.
  voice_hash = { for name in keys(var.voice_servers) : name => parseint(substr(sha256(name), 0, 8), 16) }
  voice_placement = {
    for name, hash in local.voice_hash : name => {
      subnet = hash % length(local.zones)
      ipv6   = cidrhost(local.public_ipv6_cidrs[hash % length(local.zones)], 4294967296 + hash)
    }
  }
}

resource "aws_instance" "voice" {
  for_each               = var.voice_servers
  ami                    = data.aws_ssm_parameter.ecs_ami.value
  instance_type          = coalesce(each.value.instance_type, local.voice_instance_type)
  subnet_id              = aws_subnet.public[local.voice_placement[each.key].subnet].id
  ipv6_addresses         = [local.voice_placement[each.key].ipv6]
  vpc_security_group_ids = [local.sg["voice"]]
  iam_instance_profile   = aws_iam_instance_profile.voice.name
  tags                   = { Name = "${var.name}-${each.key}" }

  # The attribute is what its service places its one task by. The voice server shares the
  # machine's network, so its user is refused the instance metadata service at every boot
  # (voice-imds.service says why).
  user_data = <<-EOT
    #!/bin/bash
    cat >> /etc/ecs/ecs.config <<'CONFIG'
    ECS_CLUSTER=${module.cluster.name}
    ECS_INSTANCE_ATTRIBUTES={"aspen.voice":"${each.key}"}
    CONFIG
    echo '${filebase64("${path.module}/../aws-single/machine/voice-imds.service")}' | base64 -d > /etc/systemd/system/aspen-voice-imds.service
    systemctl enable --now aspen-voice-imds.service
  EOT

  metadata_options {
    http_tokens = "required"
  }

  root_block_device {
    encrypted = true
  }

  lifecycle {
    # A newer recommended image would otherwise replace every voice server, ending its calls,
    # on whichever apply came next. To move one to the current image, replace it by name:
    # tofu apply -replace='aws_instance.voice["voice-1"]'
    ignore_changes = [ami]
  }
}

resource "aws_eip_association" "voice" {
  for_each      = var.voice_servers
  instance_id   = aws_instance.voice[each.key].id
  allocation_id = aws_eip.voice[each.key].id
}

resource "aws_ecs_task_definition" "voice" {
  for_each                 = var.voice_servers
  family                   = "${var.name}-${each.key}"
  requires_compatibilities = ["EC2"]
  # The host's own network: the media ports are a range, taken as calls need them.
  network_mode       = "host"
  execution_role_arn = module.cluster.execution_role_arns["voice-${each.key}"]

  container_definitions = jsonencode([{
    name              = "voice"
    image             = module.cluster.image["voice"]
    essential         = true
    memoryReservation = 512
    entryPoint        = ["sh", "-ec"]
    command = [<<-EOT
      umask 077
      printf '%s' "$VOICE_SERVER_TOML" > voice_server.toml
      unset VOICE_SERVER_TOML
      exec voice_server
    EOT
    ]
    secrets          = [{ name = "VOICE_SERVER_TOML", valueFrom = local.config_secret["voice-${each.key}"] }]
    portMappings     = [{ containerPort = 9000, hostPort = 9000, protocol = "tcp" }]
    ulimits          = [{ name = "nofile", softLimit = 65536, hardLimit = 65536 }]
    logConfiguration = module.cluster.log_configuration["voice"]
  }])
}

resource "aws_lb_target_group" "voice" {
  for_each    = var.voice_servers
  vpc_id      = aws_vpc.this.id
  target_type = "instance"
  protocol    = "HTTP"
  port        = 9000
  tags        = { Name = "${var.name}-${each.key}" }

  deregistration_delay = 10

  health_check {
    path                = "/health"
    matcher             = "204"
    interval            = 15
    timeout             = 5
    healthy_threshold   = 2
    unhealthy_threshold = 3
  }
}

resource "aws_lb_listener_rule" "voice" {
  for_each     = var.voice_servers
  listener_arn = aws_lb_listener.https.arn

  condition {
    host_header {
      values = ["${each.key}.${var.domain}"]
    }
  }

  action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.voice[each.key].arn
  }
}

resource "aws_route53_record" "voice" {
  for_each = { for pair in setproduct(keys(var.voice_servers), ["A", "AAAA"]) : "${pair[0]} ${pair[1]}" => pair }
  zone_id  = var.route53_zone_id
  name     = "${each.value[0]}.${var.domain}"
  type     = each.value[1]

  alias {
    name                   = aws_lb.this.dns_name
    zone_id                = aws_lb.this.zone_id
    evaluate_target_health = true
  }
}

resource "aws_ecs_service" "voice" {
  for_each        = var.voice_servers
  name            = each.key
  cluster         = module.cluster.id
  task_definition = aws_ecs_task_definition.voice[each.key].arn
  launch_type     = "EC2"
  desired_count   = local.deployed ? 1 : 0

  # One task on one machine's ports: the old one stops before the new one starts. Its calls
  # end, and their clients rejoin on their own.
  deployment_minimum_healthy_percent = 0
  deployment_maximum_percent         = 100

  placement_constraints {
    type       = "memberOf"
    expression = "attribute:aspen.voice == ${each.key}"
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.voice[each.key].arn
    container_name   = "voice"
    container_port   = 9000
  }

  # A voice server stops at startup when its id is not registered, which the release does.
  depends_on = [aws_lb_listener_rule.voice, aws_ecs_service.nats, aws_eip_association.voice, terraform_data.release]
}
