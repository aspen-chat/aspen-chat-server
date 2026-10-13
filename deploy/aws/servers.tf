# The API servers, the private workers, and the release task: one image and one aspen.toml,
# started three ways.

locals {
  # Every container of the image starts through this: it writes the configuration into
  # /etc/aspen, the image's working directory, which only its user may read, and then runs the
  # container's command. A command given to one task in its place (run_task.py) starts the
  # same way, which is how operator commands find the deployment's aspen.toml.
  server_entry_point = ["sh", "-ec", <<-EOT
    umask 077
    printf '%s' "$ASPEN_TOML" > aspen.toml
    printf '%s' "$RDS_CA" > ${local.rds_ca_path}
    unset ASPEN_TOML RDS_CA
    exec "$@"
  EOT
  , "aspen"]

  server_commands = {
    api    = ["aspen-chat-server", "--no-https", "--listen-addr", "0.0.0.0", "--port", "8080"]
    worker = ["aspen-chat-server", "--private-worker"]
    # Before any server of a release starts: the tables it expects, and the voice servers this
    # configuration declares, each under the id its NATS user and its own configuration name.
    release = ["sh", "-ec", join("\n", concat(["aspen-migrate up"], [
      for name, voice in var.voice_servers :
      "aspen-chat-server voice-servers add ${name} --url https://${name}.${var.domain} --capacity ${voice.capacity} --id ${random_uuid.voice[name].result}"
    ]))]
  }

  server_sizes = {
    api     = { cpu = var.api.cpu, memory = var.api.memory }
    worker  = { cpu = var.workers.cpu, memory = var.workers.memory }
    release = { cpu = 512, memory = 1024 }
  }
}

resource "aws_ecs_task_definition" "server" {
  for_each                 = local.server_commands
  family                   = "${var.name}-${each.key}"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = local.server_sizes[each.key].cpu
  memory                   = local.server_sizes[each.key].memory
  execution_role_arn       = aws_iam_role.execution.arn

  runtime_platform {
    operating_system_family = local.runtime_platform.operating_system_family
    cpu_architecture        = local.runtime_platform.cpu_architecture
  }

  container_definitions = jsonencode([{
    name       = "aspen"
    image      = local.image["api"]
    essential  = true
    entryPoint = local.server_entry_point
    command    = each.value
    secrets    = [{ name = "ASPEN_TOML", valueFrom = local.config_secret["aspen-toml"] }]
    environment = concat(
      [{ name = "RDS_CA", value = data.http.rds_ca.response_body }],
      [for name, value in var.server_environment : { name = name, value = value }],
    )
    portMappings = each.key == "api" ? [{ containerPort = 8080, protocol = "tcp" }] : []
    # Every connection and event stream is a file; Fargate's default allows 65535, fewer than
    # `[connections] max`.
    ulimits          = [{ name = "nofile", softLimit = 262144, hardLimit = 262144 }]
    logConfiguration = local.log_configuration[each.key]
  }])
}

resource "aws_lb" "this" {
  name               = var.name
  load_balancer_type = "application"
  subnets            = aws_subnet.public[*].id
  security_groups    = [local.sg["load-balancer"]]

  # Below `[connections] idle_seconds` (120), so the servers never close a connection the
  # load balancer is about to reuse. Event streams are kept open by the servers' pings, every
  # 30 seconds.
  idle_timeout               = 60
  drop_invalid_header_fields = true
}

resource "aws_lb_target_group" "api" {
  name        = "${var.name}-api"
  vpc_id      = aws_vpc.this.id
  target_type = "ip"
  protocol    = "HTTP"
  port        = 8080

  deregistration_delay = 30

  health_check {
    # Needs no session, and is answered from the server's copy of the deployment settings.
    path                = "/api/v1/deployment"
    matcher             = "200"
    interval            = 15
    timeout             = 5
    healthy_threshold   = 2
    unhealthy_threshold = 3
  }
}

resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.this.arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = "ELBSecurityPolicy-TLS13-1-2-2021-06"
  certificate_arn   = aws_acm_certificate_validation.deployment.certificate_arn

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.api.arn
  }
}

resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.this.arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type = "redirect"

    redirect {
      protocol    = "HTTPS"
      port        = "443"
      status_code = "HTTP_301"
    }
  }
}

resource "aws_ecs_service" "api" {
  name            = "api"
  cluster         = aws_ecs_cluster.this.id
  task_definition = aws_ecs_task_definition.server["api"].arn
  launch_type     = "FARGATE"
  desired_count   = local.deployed ? var.api.count : 0

  # New servers take requests before the old ones stop. People's event streams reconnect to
  # them by themselves and resume where they left off.
  deployment_minimum_healthy_percent = 100
  deployment_maximum_percent         = 200
  health_check_grace_period_seconds  = 60

  network_configuration {
    subnets          = aws_subnet.public[*].id
    security_groups  = [local.sg["server"]]
    assign_public_ip = true
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.api.arn
    container_name   = "aspen"
    container_port   = 8080
  }

  depends_on = [aws_lb_listener.https, aws_ecs_service.nats, terraform_data.release]
}

resource "aws_ecs_service" "worker" {
  name            = "worker"
  cluster         = aws_ecs_cluster.this.id
  task_definition = aws_ecs_task_definition.server["worker"].arn
  launch_type     = "FARGATE"
  desired_count   = local.deployed ? var.workers.count : 0

  network_configuration {
    subnets          = aws_subnet.public[*].id
    security_groups  = [local.sg["server"]]
    assign_public_ip = true
  }

  depends_on = [aws_ecs_service.nats, terraform_data.release]
}
