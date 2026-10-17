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
  description = "Where Let's Encrypt writes about the deployment's certificates expiring, given to the box when it is made. Null gives it no address."
  type        = string
  default     = null
}

variable "image_tag" {
  description = "The release to run, a tag of both images. Changing it upgrades the box (`aspen-box upgrade`)."
  type        = string
}

variable "image_repositories" {
  description = "Where the images are pulled from. The published ones by default; the machine pulls without signing in."
  type = object({
    api   = optional(string, "ghcr.io/aspen-chat/aspen-api")
    voice = optional(string, "ghcr.io/aspen-chat/aspen-voice")
  })
  default = {}
}

variable "cpu_architecture" {
  description = "The machine's: X86_64 or ARM64 (Graviton, which costs about a tenth less). The published images are made for both."
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
  description = "The volume the box is kept on (its configuration, the database, the certificates, and its backups), which outlives the machine. It can be grown, never shrunk."
  type        = number
  default     = 20
}

variable "backup_retention_days" {
  description = "How many daily snapshots of the data volume are kept."
  type        = number
  default     = 7
}
