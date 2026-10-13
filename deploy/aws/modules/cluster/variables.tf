variable "name" {
  description = "The deployment's name."
  type        = string
}

variable "region" {
  type = string
}

variable "image_tag" {
  description = "The tag of the images to run, or null before any is pushed."
  type        = string
}

variable "log_retention_days" {
  type = number
}

variable "log_roles" {
  description = "The kinds of container, each with log streams of its own."
  type        = list(string)
}

variable "config_secret_arns" {
  description = "The secrets holding the configuration files tasks are started with."
  type        = list(string)
}
