locals {
  zones = [for suffix in var.availability_zone_suffixes : "${var.region}${suffix}"]

  # Public subnets from the bottom of the network and private ones from 128 up, a /20 each.
  public_cidrs  = [for index, _ in local.zones : cidrsubnet(var.vpc_cidr, 4, index)]
  private_cidrs = [for index, _ in local.zones : cidrsubnet(var.vpc_cidr, 4, index + 8)]
}

resource "aws_vpc" "this" {
  cidr_block           = var.vpc_cidr
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = { Name = var.name }
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = var.name }
}

# The servers are in public subnets with public addresses of their own, and security groups
# that take nothing from the internet: the API servers fetch link previews, other deployments,
# and push services directly (docs/operators/installing/4-api-server.md), which from a private
# subnet would need a NAT gateway in each zone.
resource "aws_subnet" "public" {
  count                   = length(local.zones)
  vpc_id                  = aws_vpc.this.id
  availability_zone       = local.zones[count.index]
  cidr_block              = local.public_cidrs[count.index]
  map_public_ip_on_launch = true
  tags                    = { Name = "${var.name}-public-${var.availability_zone_suffixes[count.index]}" }
}

# The database and Valkey, with no route out of the network.
resource "aws_subnet" "private" {
  count             = length(local.zones)
  vpc_id            = aws_vpc.this.id
  availability_zone = local.zones[count.index]
  cidr_block        = local.private_cidrs[count.index]
  tags              = { Name = "${var.name}-private-${var.availability_zone_suffixes[count.index]}" }
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.this.id
  tags   = { Name = "${var.name}-public" }

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.this.id
  }
}

resource "aws_route_table_association" "public" {
  count          = length(local.zones)
  subnet_id      = aws_subnet.public[count.index].id
  route_table_id = aws_route_table.public.id
}

# One security group per role. Each rule below names the group it lets in, so what may reach
# what (docs/operators/installing/network-exposure.md) reads off this file.
resource "aws_security_group" "role" {
  for_each    = toset(["load-balancer", "server", "nats", "database", "valkey", "voice"])
  name        = "${var.name}-${each.key}"
  description = "Aspen ${var.name}: ${each.key}"
  vpc_id      = aws_vpc.this.id
  tags        = { Name = "${var.name}-${each.key}" }
}

locals {
  sg = { for role, group in aws_security_group.role : role => group.id }

  # From, to, and port, for traffic inside the network.
  internal_rules = {
    load-balancer-to-server = ["load-balancer", "server", 8080]
    load-balancer-to-voice  = ["load-balancer", "voice", 9000]
    server-to-nats          = ["server", "nats", 4222]
    # NATS's monitoring port, which the API servers size its streams from and nothing else
    # may reach.
    server-to-nats-monitor = ["server", "nats", 8222]
    voice-to-nats          = ["voice", "nats", 4222]
    server-to-database     = ["server", "database", 5432]
    server-to-valkey       = ["server", "valkey", 6379]
  }

  # Protocol and ports open to everyone.
  public_rules = {
    load-balancer-http   = ["load-balancer", "tcp", 80, 80]
    load-balancer-https  = ["load-balancer", "tcp", 443, 443]
    voice-media-udp      = ["voice", "udp", var.voice_media_ports.min, var.voice_media_ports.max]
    voice-media-tcp      = ["voice", "tcp", var.voice_media_ports.min, var.voice_media_ports.max]
    voice-file-transfers = ["voice", "udp", 3478, 3478]
  }
}

resource "aws_vpc_security_group_ingress_rule" "internal" {
  for_each                     = local.internal_rules
  security_group_id            = local.sg[each.value[1]]
  referenced_security_group_id = local.sg[each.value[0]]
  ip_protocol                  = "tcp"
  from_port                    = each.value[2]
  to_port                      = each.value[2]
  description                  = each.key
}

resource "aws_vpc_security_group_ingress_rule" "public" {
  for_each          = local.public_rules
  security_group_id = local.sg[each.value[0]]
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = each.value[1]
  from_port         = each.value[2]
  to_port           = each.value[3]
  description       = each.key
}

# Everything may open connections outward: the servers to the services and the internet, and
# all of them to pull their images. The database and Valkey have no route to use it.
resource "aws_vpc_security_group_egress_rule" "all" {
  for_each          = local.sg
  security_group_id = each.value
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = "-1"
}
