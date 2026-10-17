variable "name" {
  description = "Names every resource of this deployment, so several can share an account."
  type        = string
  default     = "aspen"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{1,20}$", var.name))
    error_message = "Use 2 to 21 lowercase letters, digits, and hyphens, starting with a letter."
  }
}

variable "region" {
  description = "The AWS region everything but the media certificate is made in."
  type        = string
}

variable "availability_zone_suffixes" {
  description = "The region's availability zones to spread over, by their letters. The load balancer, the database's subnet group, and Valkey's each need two."
  type        = list(string)
  default     = ["a", "b"]

  validation {
    condition     = length(var.availability_zone_suffixes) >= 2
    error_message = "Name at least two availability zones."
  }
}

variable "vpc_cidr" {
  description = "The private network's addresses. A /16, of which each subnet takes a /20."
  type        = string
  default     = "10.42.0.0/16"
}

variable "domain" {
  description = "The deployment's address without a scheme (chat.example.org): `public_url` is https:// and this, for good (docs/operators/installing/3-configuring.md). Media is served at media. under it and each voice server at its name under it."
  type        = string
}

variable "route53_zone_id" {
  description = "The Route 53 hosted zone that holds `domain`, where the deployment's records and its certificates' validation records are written."
  type        = string
}

variable "public_dns" {
  description = "Whether `domain` points at this deployment. False while it is made ready beside the deployment it replaces, which keeps the name until the move (README.md#moving-from-one-machine)."
  type        = bool
  default     = true
}

variable "media_bucket" {
  description = "The bucket of the deployment this one replaced, kept and imported rather than copied (README.md#moving-from-one-machine). Null makes a bucket of this deployment's own. Once set it stays set: it is the bucket's name."
  type        = string
  default     = null
}

variable "image_tag" {
  description = "The release to run, a tag of both images. Null runs no servers: a deployment made ready beside the one it replaces, before its database is copied (README.md#moving-from-one-machine)."
  type        = string
  default     = null
}

variable "image_repositories" {
  description = "Where the images are pulled from. The published ones by default; images of your own must be in a repository ECS may pull from without signing in, or in ECR."
  type = object({
    api   = optional(string, "ghcr.io/aspen-chat/aspen-api")
    voice = optional(string, "ghcr.io/aspen-chat/aspen-voice")
  })
  default = {}
}

variable "cpu_architecture" {
  description = "What the servers run on: X86_64 or ARM64 (Graviton, which costs less). The published images are made for both."
  type        = string
  default     = "X86_64"

  validation {
    condition     = contains(["X86_64", "ARM64"], var.cpu_architecture)
    error_message = "X86_64 or ARM64."
  }
}

variable "api" {
  description = "The API servers: how many, and each one's CPU units (1024 is one vCPU) and memory in MiB, a pair Fargate offers."
  type = object({
    count  = optional(number, 2)
    cpu    = optional(number, 1024)
    memory = optional(number, 2048)
  })
  default = {}
}

variable "workers" {
  description = "Private workers (docs/operators/installing/4-api-server.md), which take the background work without serving requests. None by default: every API server does a share."
  type = object({
    count  = optional(number, 0)
    cpu    = optional(number, 1024)
    memory = optional(number, 2048)
  })
  default = {}
}

variable "server_environment" {
  description = "Settings for the API servers and workers beyond those this configuration writes, as the environment variables that override aspen.toml (ASPEN_EVENT_QUEUE_SIZE, ASPEN_AUTH__REVERIFY_SECONDS)."
  type        = map(string)
  default     = {}
}

variable "email" {
  description = "The SMTP server mail is sent through and the address it is from. Null sends no mail."
  type = object({
    smtp_url = string
    from     = string
  })
  default   = null
  sensitive = true
}

variable "nats" {
  description = "The NATS task's CPU units and memory in MiB. Five eighths of the memory is JetStream's to reserve for the event and voice report streams, which ask for 1152 MiB together unless the deployment settings say otherwise."
  type = object({
    cpu    = optional(number, 512)
    memory = optional(number, 2048)
  })
  default = {}
}

variable "database" {
  description = "The PostgreSQL instance. The migrations need PostgreSQL 18 or later."
  type = object({
    instance_class        = optional(string, "db.t4g.small")
    engine_version        = optional(string, "18")
    allocated_storage     = optional(number, 20)
    max_allocated_storage = optional(number, 200)
    multi_az              = optional(bool, false)
    backup_retention_days = optional(number, 7)
    deletion_protection   = optional(bool, true)
  })
  default = {}
}

variable "valkey_node_type" {
  description = "The Valkey node. Aspen needs about half a gigabyte (docs/operators/installing/1-services.md)."
  type        = string
  default     = "cache.t4g.small"
}

variable "voice_servers" {
  description = "The voice servers by name, each a machine of its own at <name>.<domain>. `capacity` is the most participants it carries, as `voice_server estimate-capacity` measures on that instance type."
  type = map(object({
    capacity      = number
    instance_type = optional(string)
  }))
  default = {}

  validation {
    condition     = alltrue([for name in keys(var.voice_servers) : can(regex("^[a-z][a-z0-9-]{0,30}$", name)) && name != "media"])
    error_message = "A voice server's name is a DNS label under the domain: lowercase letters, digits, and hyphens, and not `media`."
  }
}

variable "voice_media_ports" {
  description = "The UDP and TCP ports each voice server takes media on ([rtc] min_port and max_port). A call's participant takes one."
  type = object({
    min = optional(number, 40000)
    max = optional(number, 40999)
  })
  default = {}
}

variable "log_retention_days" {
  type    = number
  default = 30
}
