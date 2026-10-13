locals {
  media_domain = "media.${var.domain}"
}

# The deployment's own address and every voice server's, for the load balancer.
resource "aws_acm_certificate" "deployment" {
  domain_name               = var.domain
  subject_alternative_names = ["*.${var.domain}"]
  validation_method         = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_acm_certificate" "media" {
  provider          = aws.us_east_1
  domain_name       = local.media_domain
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

# Keyed by certificate rather than by validation option, whose names are known only once the
# certificate is made. A name and its wildcard are validated by one record, the name's.
resource "aws_route53_record" "certificate_validation" {
  for_each = {
    deployment = { domain = var.domain, options = aws_acm_certificate.deployment.domain_validation_options }
    media      = { domain = local.media_domain, options = aws_acm_certificate.media.domain_validation_options }
  }

  zone_id         = var.route53_zone_id
  name            = one([for option in each.value.options : option.resource_record_name if option.domain_name == each.value.domain])
  type            = one([for option in each.value.options : option.resource_record_type if option.domain_name == each.value.domain])
  records         = [one([for option in each.value.options : option.resource_record_value if option.domain_name == each.value.domain])]
  ttl             = 60
  allow_overwrite = true
}

resource "aws_acm_certificate_validation" "deployment" {
  certificate_arn         = aws_acm_certificate.deployment.arn
  validation_record_fqdns = [aws_route53_record.certificate_validation["deployment"].fqdn]
}

resource "aws_acm_certificate_validation" "media" {
  provider                = aws.us_east_1
  certificate_arn         = aws_acm_certificate.media.arn
  validation_record_fqdns = [aws_route53_record.certificate_validation["media"].fqdn]
}

resource "aws_route53_record" "deployment" {
  zone_id = var.route53_zone_id
  name    = var.domain
  type    = "A"

  alias {
    name                   = aws_lb.this.dns_name
    zone_id                = aws_lb.this.zone_id
    evaluate_target_health = true
  }
}

resource "aws_route53_record" "media" {
  zone_id = var.route53_zone_id
  name    = local.media_domain
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.media.domain_name
    zone_id                = aws_cloudfront_distribution.media.hosted_zone_id
    evaluate_target_health = false
  }
}
