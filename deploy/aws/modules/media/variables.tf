variable "name" {
  description = "The deployment's name, which names the bucket and everything beside it."
  type        = string
}

variable "domain" {
  description = "The read path's address (media.chat.example.org), an origin of its own."
  type        = string
}

variable "route53_zone_id" {
  description = "The hosted zone that holds `domain`."
  type        = string
}

variable "public_dns" {
  description = "Whether `domain` points at the read path. False while the deployment this one replaces still answers there."
  type        = bool
  default     = true
}
