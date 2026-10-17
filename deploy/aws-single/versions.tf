terraform {
  required_version = ">= 1.8"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }
}

provider "aws" {
  region = var.region

  default_tags {
    tags = { Deployment = var.name }
  }
}

# CloudFront takes certificates from us-east-1 alone, whichever region the deployment is in.
provider "aws" {
  alias  = "us_east_1"
  region = "us-east-1"

  default_tags {
    tags = { Deployment = var.name }
  }
}
