# NATS, one task: its streams are in memory and hold a minute of events, so there is nothing
# to keep when it is replaced. The servers find it by name and make its streams again when it
# comes back without them.
resource "aws_service_discovery_private_dns_namespace" "this" {
  name = "${var.name}.internal"
  vpc  = aws_vpc.this.id
}

resource "aws_service_discovery_service" "nats" {
  name = "nats"

  dns_config {
    namespace_id   = aws_service_discovery_private_dns_namespace.this.id
    routing_policy = "MULTIVALUE"

    dns_records {
      type = "A"
      ttl  = 10
    }
  }
}

resource "aws_ecs_task_definition" "nats" {
  family                   = "${var.name}-nats"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.nats.cpu
  memory                   = var.nats.memory
  execution_role_arn       = module.cluster.execution_role_arns["nats"]

  runtime_platform {
    operating_system_family = local.runtime_platform.operating_system_family
    cpu_architecture        = local.runtime_platform.cpu_architecture
  }

  container_definitions = jsonencode([{
    name       = "nats"
    image      = "public.ecr.aws/docker/library/nats:2.11-alpine"
    essential  = true
    entryPoint = ["sh", "-ec"]
    command = [<<-EOT
      umask 077
      printf '%s' "$NATS_CONF" > /tmp/nats.conf
      unset NATS_CONF
      exec nats-server --config /tmp/nats.conf
    EOT
    ]
    secrets = [{ name = "NATS_CONF", valueFrom = local.config_secret["nats-conf"] }]
    portMappings = [
      { containerPort = 4222, protocol = "tcp" },
      { containerPort = 8222, protocol = "tcp" },
    ]
    # The image's own BusyBox fetches NATS's health from its monitoring port.
    healthCheck = {
      command     = ["CMD", "wget", "-q", "-O", "/dev/null", "http://127.0.0.1:8222/healthz"]
      interval    = 15
      timeout     = 5
      retries     = 3
      startPeriod = 15
    }
    logConfiguration = module.cluster.log_configuration["nats"]
  }])
}

resource "aws_ecs_service" "nats" {
  name            = "nats"
  cluster         = module.cluster.id
  task_definition = aws_ecs_task_definition.nats.arn
  launch_type     = "FARGATE"
  desired_count   = 1

  # The old task stops before the new one starts: two at once would each hold some of the
  # servers, and events published to one would not reach the other's.
  deployment_minimum_healthy_percent = 0
  deployment_maximum_percent         = 100

  # An apply goes on only once NATS is up again: the release's commands, which follow, sign in
  # to it.
  wait_for_steady_state = true

  network_configuration {
    subnets          = aws_subnet.public[*].id
    security_groups  = [local.sg["nats"]]
    assign_public_ip = true
  }

  service_registries {
    registry_arn = aws_service_discovery_service.nats.arn
  }
}
