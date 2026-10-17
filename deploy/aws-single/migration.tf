# Leaving for the larger configuration (../aws/README.md#moving-a-box-there): what
# ../aws/migrate.py reads. The bucket and its read path stay where they are, and that
# deployment takes them over; only the database is copied (`aspen-box export --database-only`).

output "handover" {
  description = "What ../aws/migrate.py reads to move this deployment to the larger configuration."
  value = {
    name            = var.name
    region          = var.region
    domain          = var.domain
    route53_zone_id = var.route53_zone_id
    instance        = aws_instance.this.id
    bucket          = module.media.bucket
    dump            = module.media.migration_dump.uri
    distribution_id = module.media.distribution_id
    # Forgotten rather than destroyed when this deployment is taken down: the storage the
    # larger one goes on using, the name that now points at it, and the data volume, kept as
    # the way back.
    kept = concat(
      [for address in module.media.handed_over : "module.media.${address}"],
      [for type in ["A", "AAAA"] : "aws_route53_record.machine[\"${var.domain} ${type}\"]"],
      ["aws_ebs_volume.data"],
    )
    data_volume = aws_ebs_volume.data.id
  }
}
