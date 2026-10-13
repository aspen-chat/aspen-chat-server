module "media" {
  source = "./modules/media"

  name            = var.name
  domain          = "media.${var.domain}"
  route53_zone_id = var.route53_zone_id

  providers = {
    aws           = aws
    aws.us_east_1 = aws.us_east_1
  }
}
