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
  type = string
}

variable "availability_zone_suffix" {
  description = "The letter of the region's availability zone the machine and its data volume are in."
  type        = string
  default     = "a"
}

variable "domain" {
  description = "The deployment's address without a scheme (chat.example.org): `public_url` is https:// and this, for good (docs/operators/installing/3-configuring.md). Media is served at media. under it and the voice server at voice. under it."
  type        = string
}

variable "route53_zone_id" {
  description = "The Route 53 hosted zone that holds `domain`."
  type        = string
}

variable "acme_email" {
  description = "Where Let's Encrypt writes about the deployment's certificates expiring. Null gives it no address."
  type        = string
  default     = null
}

variable "image_tag" {
  description = "The tag of the images to run, as pushed to the two repositories this makes. Null runs neither server: the first apply, before there is a repository to push to."
  type        = string
  default     = null
}

variable "cpu_architecture" {
  description = "What the images were built for: X86_64 or ARM64 (Graviton, which costs about a tenth less; build the images on an ARM machine)."
  type        = string
  default     = "X86_64"

  validation {
    condition     = contains(["X86_64", "ARM64"], var.cpu_architecture)
    error_message = "X86_64 or ARM64."
  }
}

variable "instance_type" {
  description = "The machine. Null is the architecture's instance with 2 vCPUs and 4 GiB (t3a.medium or t4g.medium). A reserved instance or savings plan bought for the same type is what makes it cheap; this configuration buys neither."
  type        = string
  default     = null
}

variable "data_volume_gb" {
  description = "The volume the database and the certificates are kept on, which outlives the machine. It can be grown, never shrunk."
  type        = number
  default     = 20
}

variable "backup_retention_days" {
  description = "How many daily snapshots of the data volume are kept."
  type        = number
  default     = 7
}

variable "nats_stream_mib" {
  description = "The memory NATS may reserve for the event and voice report streams, which the servers size to it. Each needs at least 16."
  type        = number
  default     = 256

  validation {
    condition     = var.nats_stream_mib >= 144
    error_message = "Voice reports take an eighth, which must be at least 16 MiB: give NATS 144 MiB or more."
  }
}

variable "valkey_memory_mb" {
  type    = number
  default = 128
}

variable "voice_capacity" {
  description = "The most participants the voice server carries at once. The default is a guess for 2 vCPUs: `voice_server estimate-capacity`, run on the machine, measures it."
  type        = number
  default     = 25
}

variable "voice_media_ports" {
  description = "The UDP and TCP ports the voice server takes media on ([rtc] min_port and max_port). A call's participant takes one."
  type = object({
    min = optional(number, 40000)
    max = optional(number, 40199)
  })
  default = {}
}

variable "server_environment" {
  description = "Settings for the API server beyond those this configuration writes, as the environment variables that override aspen.toml (ASPEN_EVENT_QUEUE_SIZE)."
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

variable "log_retention_days" {
  type    = number
  default = 14
}
