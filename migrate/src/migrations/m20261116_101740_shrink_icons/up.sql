-- One `shrinkIcons` job (`app::icon::shrink_step`) stores every icon confirmed before now as
-- icons are now stored on confirming, fitted within their box as WebP, in bounded steps.
INSERT INTO job (id, kind, key, class, due, not_before, payload)
VALUES (gen_random_uuid(), 'shrinkIcons', NULL, 4, now(), now(), '{}');
