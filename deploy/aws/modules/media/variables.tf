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

variable "bucket" {
  description = "The name of a bucket to keep rather than name one after this deployment: the bucket of the deployment this one replaces, imported into this module. Null makes a bucket."
  type        = string
  default     = null
}
