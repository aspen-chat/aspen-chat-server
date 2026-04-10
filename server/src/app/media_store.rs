use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;

use crate::app;
use crate::aspen_config::AspenConfig;

#[derive(Clone)]
pub struct MediaStore {
    client: Client,
    bucket: String,
}

impl MediaStore {
    pub async fn new(config: &AspenConfig) -> app::error::Result<Self> {
        let s3 = &config.media.s3;
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(s3.region.clone()))
            .credentials_provider(Credentials::new(
                s3.access_key.clone(),
                s3.secret_key.clone(),
                None,
                None,
                "aspen-media-config",
            ))
            .load()
            .await;
        let s3_config = aws_sdk_s3::config::Builder::from(&sdk_config)
            .endpoint_url(s3.endpoint.clone())
            .force_path_style(true)
            .build();
        let client = Client::from_conf(s3_config);
        Ok(Self {
            client,
            bucket: s3.bucket.clone(),
        })
    }

    pub async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> app::error::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(Box::new)?;
        Ok(())
    }

    pub async fn get_bytes(&self, key: &str) -> app::error::Result<Vec<u8>> {
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(Box::new)?;
        let body = response.body.collect().await?;
        Ok(body.into_bytes().to_vec())
    }

    pub async fn delete(&self, key: &str) -> app::error::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(Box::new)?;
        Ok(())
    }
}
