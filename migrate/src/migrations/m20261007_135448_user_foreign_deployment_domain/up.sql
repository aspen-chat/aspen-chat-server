-- Which of this deployment's users use a deployment, by that deployment, so whether it is in
-- use is read without reading every row (`app::federation::directory::IN_USE_SQL`).
CREATE INDEX user_foreign_deployment_domain ON user_foreign_deployment (domain);
