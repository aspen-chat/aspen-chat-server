-- A transfer ends `notPermitted` when its sender loses Transfer files in the call.
ALTER TABLE file_transfer DROP CONSTRAINT file_transfer_outcome_check;
ALTER TABLE file_transfer ADD CONSTRAINT file_transfer_outcome_check
    CHECK (outcome IN ('completed', 'cancelled', 'failed', 'left', 'notPermitted'));
