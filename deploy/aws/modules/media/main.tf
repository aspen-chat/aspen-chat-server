# A deployment's object storage: the bucket, what the servers' role may do with it
# (`access_policy`), and the read path clients download from. Both AWS configurations use it.

terraform {
  required_providers {
    aws = {
      source                = "hashicorp/aws"
      configuration_aliases = [aws.us_east_1]
    }
  }
}

# A bucket's name is for good: changing it means a new bucket. `var.bucket` keeps one made
# under another deployment's name, when this deployment takes that one's storage over.
resource "aws_s3_bucket" "media" {
  bucket        = var.bucket
  bucket_prefix = var.bucket == null ? "${var.name}-media-" : null
}

# `migration/` holds a dump of the database while a deployment moves from one configuration to
# another (../../migrate.py). The read path does not serve it, and a dump is every
# account's data, so one is not left there.
resource "aws_s3_bucket_lifecycle_configuration" "media" {
  bucket = aws_s3_bucket.media.id

  rule {
    id     = "expire-migration-dumps"
    status = "Enabled"

    filter {
      prefix = "${local.migration_prefix}/"
    }

    expiration {
      days = 7
    }
  }
}

resource "aws_s3_bucket_public_access_block" "media" {
  bucket                  = aws_s3_bucket.media.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# Clients upload straight to the bucket, to URLs the server signs for one object each
# ([media.s3] upload_url_ttl_seconds). The signature is what allows an upload, so any origin
# may send one:
# the web client's, and the desktop and mobile apps', whose origins are `null`,
# `capacitor://localhost`, and `https://localhost`.
resource "aws_s3_bucket_cors_configuration" "media" {
  bucket = aws_s3_bucket.media.id

  cors_rule {
    allowed_methods = ["PUT"]
    allowed_origins = ["*"]
    allowed_headers = ["*"]
    max_age_seconds = 3600
  }
}

# The read path (docs/operators/installing/storage-read-path.md): CloudFront at an origin of
# its own, which the bucket lets read the four prefixes clients read and nothing else. It has
# no permission to list, so `evidence/`, `uploads/`, and the bucket's root answer 403.
locals {
  public_prefixes  = ["attachments", "attachment-previews", "icons", "link-preview-images"]
  migration_prefix = "migration"
}

resource "aws_cloudfront_origin_access_control" "media" {
  name                              = "${var.name}-media"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_cloudfront_response_headers_policy" "media" {
  name = "${var.name}-media"

  security_headers_config {
    content_type_options {
      override = true
    }
  }
}

resource "aws_cloudfront_distribution" "media" {
  enabled         = true
  is_ipv6_enabled = true
  comment         = "Aspen ${var.name}: media downloads"
  aliases         = [var.domain]
  price_class     = "PriceClass_100"
  http_version    = "http2and3"

  origin {
    origin_id                = "media"
    domain_name              = aws_s3_bucket.media.bucket_regional_domain_name
    origin_access_control_id = aws_cloudfront_origin_access_control.media.id
  }

  default_cache_behavior {
    target_origin_id       = "media"
    viewer_protocol_policy = "redirect-to-https"
    allowed_methods        = ["GET", "HEAD"]
    cached_methods         = ["GET", "HEAD"]
    compress               = true
    # AWS's managed CachingOptimized policy: objects are named by ids and never change.
    cache_policy_id            = "658327ea-f89d-4fab-a63d-7e88639e58f6"
    response_headers_policy_id = aws_cloudfront_response_headers_policy.media.id
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn      = aws_acm_certificate_validation.this.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

resource "aws_s3_bucket_policy" "media" {
  bucket = aws_s3_bucket.media.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Sid       = "ReadPathThroughCloudFront"
      Effect    = "Allow"
      Principal = { Service = "cloudfront.amazonaws.com" }
      Action    = "s3:GetObject"
      Resource  = [for prefix in local.public_prefixes : "${aws_s3_bucket.media.arn}/${prefix}/*"]
      Condition = { StringEquals = { "AWS:SourceArn" = aws_cloudfront_distribution.media.arn } }
    }]
  })

  depends_on = [aws_s3_bucket_public_access_block.media]
}

# CloudFront takes certificates from us-east-1 alone, whichever region the bucket is in.
resource "aws_acm_certificate" "this" {
  provider          = aws.us_east_1
  domain_name       = var.domain
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_route53_record" "certificate_validation" {
  zone_id         = var.route53_zone_id
  name            = one(aws_acm_certificate.this.domain_validation_options[*].resource_record_name)
  type            = one(aws_acm_certificate.this.domain_validation_options[*].resource_record_type)
  records         = [one(aws_acm_certificate.this.domain_validation_options[*].resource_record_value)]
  ttl             = 60
  allow_overwrite = true
}

resource "aws_acm_certificate_validation" "this" {
  provider                = aws.us_east_1
  certificate_arn         = aws_acm_certificate.this.arn
  validation_record_fqdns = [aws_route53_record.certificate_validation.fqdn]
}

# Held back, by `public_dns`, while the name is still another deployment's: one this deployment
# replaces, whose storage it does not take over. One that takes it over takes this record over
# with the distribution.
resource "aws_route53_record" "this" {
  for_each        = var.public_dns ? toset(["A", "AAAA"]) : toset([])
  zone_id         = var.route53_zone_id
  name            = var.domain
  type            = each.key
  allow_overwrite = true

  alias {
    name                   = aws_cloudfront_distribution.media.domain_name
    zone_id                = aws_cloudfront_distribution.media.hosted_zone_id
    evaluate_target_health = false
  }
}
