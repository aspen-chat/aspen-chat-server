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
