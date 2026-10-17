# People's files, in a bucket of the deployment's own with CloudFront in front, as in the larger
# configuration, so a deployment that moves there takes them over in place. The box is given the
# bucket and no key pair: the server takes the machine's role.
module "media" {
  source = "../aws/modules/media"

  name            = var.name
  domain          = "media.${var.domain}"
  route53_zone_id = var.route53_zone_id

  providers = {
    aws           = aws
    aws.us_east_1 = aws.us_east_1
  }
}
