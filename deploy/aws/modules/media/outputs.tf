output "bucket" {
  value = aws_s3_bucket.media.bucket
}

output "access_key" {
  value = aws_iam_access_key.media.id
}

output "secret_key" {
  value     = aws_iam_access_key.media.secret
  sensitive = true
}

output "public_base_url" {
  description = "`[media.s3] public_base_url`."
  value       = "https://${var.domain}"
}

output "public_prefixes" {
  description = "The prefixes the read path serves."
  value       = local.public_prefixes
}
