-- A deleted category decides nothing: its channels go to no category and its overrides go.
UPDATE channel SET parent_category = NULL
WHERE parent_category IN (SELECT id FROM category WHERE deleted_at IS NOT NULL);

DELETE FROM category_override
WHERE category IN (SELECT id FROM category WHERE deleted_at IS NOT NULL);
