output "bucket" {
  value = aws_s3_bucket.media.bucket
}

output "access_policy" {
  description = "An IAM policy document of what the servers may do with the bucket, for the role they run as: read, write, and delete its objects, and list it."
  value = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload"]
        Resource = "${aws_s3_bucket.media.arn}/*"
      },
      {
        Effect   = "Allow"
        Action   = "s3:ListBucket"
        Resource = aws_s3_bucket.media.arn
      },
    ]
  })
}

output "public_base_url" {
  description = "`[media.s3] public_base_url`."
  value       = "https://${var.domain}"
}

output "public_prefixes" {
  description = "The prefixes the read path serves."
  value       = local.public_prefixes
}

output "bucket_arn" {
  value = aws_s3_bucket.media.arn
}

output "distribution_id" {
  value = aws_cloudfront_distribution.media.id
}

output "migration_dump" {
  description = "Where a database dump is put while a deployment moves to another configuration."
  value = {
    key = "${local.migration_prefix}/database.dump"
    uri = "s3://${aws_s3_bucket.media.bucket}/${local.migration_prefix}/database.dump"
    arn = "${aws_s3_bucket.media.arn}/${local.migration_prefix}/database.dump"
  }
}

output "handed_over" {
  description = "What a deployment that takes this storage over goes on using, by address in this module: the deployment giving it up forgets these rather than destroying them. The rest of the module is this deployment's own."
  value = [
    "aws_s3_bucket.media",
    "aws_s3_bucket_public_access_block.media",
    "aws_s3_bucket_cors_configuration.media",
    "aws_s3_bucket_lifecycle_configuration.media",
    "aws_s3_bucket_policy.media",
    "aws_cloudfront_distribution.media",
    "aws_route53_record.this",
    "aws_route53_record.certificate_validation",
  ]
}
