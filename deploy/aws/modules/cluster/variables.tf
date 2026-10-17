variable "name" {
  description = "The deployment's name."
  type        = string
}

variable "region" {
  type = string
}

variable "image_repositories" {
  description = "Where each of the Dockerfile's targets is pulled from, by target: `api` and `voice`."
  type        = map(string)
}

variable "image_tag" {
  description = "The tag of the images to run, or null to run none."
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
