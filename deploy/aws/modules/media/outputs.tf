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

output "cors_origins" {
  description = "The origins the read path lets read what it serves."
  value       = aws_cloudfront_response_headers_policy.media.cors_config[0].access_control_allow_origins[0].items
}

output "bucket_arn" {
  value = aws_s3_bucket.media.arn
}

output "migration_dump" {
  description = "Where a database dump is put while a deployment moves here."
  value = {
    key = "${local.migration_prefix}/database.dump"
    uri = "s3://${aws_s3_bucket.media.bucket}/${local.migration_prefix}/database.dump"
    arn = "${aws_s3_bucket.media.arn}/${local.migration_prefix}/database.dump"
  }
}
