# The deployment's own address and every voice server's, for the load balancer.
resource "aws_acm_certificate" "deployment" {
  domain_name               = var.domain
  subject_alternative_names = ["*.${var.domain}"]
  validation_method         = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

# A name and its wildcard are validated by one record, the name's.
resource "aws_route53_record" "certificate_validation" {
  zone_id         = var.route53_zone_id
  name            = one([for option in aws_acm_certificate.deployment.domain_validation_options : option.resource_record_name if option.domain_name == var.domain])
  type            = one([for option in aws_acm_certificate.deployment.domain_validation_options : option.resource_record_type if option.domain_name == var.domain])
  records         = [one([for option in aws_acm_certificate.deployment.domain_validation_options : option.resource_record_value if option.domain_name == var.domain])]
  ttl             = 60
  allow_overwrite = true
}

resource "aws_acm_certificate_validation" "deployment" {
  certificate_arn         = aws_acm_certificate.deployment.arn
  validation_record_fqdns = [aws_route53_record.certificate_validation.fqdn]
}

resource "aws_route53_record" "deployment" {
  for_each = var.public_dns ? toset(["A", "AAAA"]) : toset([])
  zone_id  = var.route53_zone_id
  name     = var.domain
  type     = each.key
  # The deployment this one replaces holds the name until this record takes it.
  allow_overwrite = true

  alias {
    name                   = aws_lb.this.dns_name
    zone_id                = aws_lb.this.zone_id
    evaluate_target_health = true
  }
}
