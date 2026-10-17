module "media" {
  source = "./modules/media"

  name            = var.name
  domain          = "media.${var.domain}"
  route53_zone_id = var.route53_zone_id
  bucket          = var.media_bucket
  # A bucket taken over comes with its read path's record. A bucket of this deployment's own
  # takes the name with the deployment's.
  public_dns = var.public_dns || var.media_bucket != null

  providers = {
    aws           = aws
    aws.us_east_1 = aws.us_east_1
  }
}

# What the API servers, the workers, and the release task may do as themselves: use the bucket.
# The server finds the role's credentials itself, given no key pair (`[media.s3]`).
resource "aws_iam_role" "server" {
  name = "${var.name}-server"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy" "server_media" {
  name   = "media"
  role   = aws_iam_role.server.id
  policy = module.media.access_policy
}
