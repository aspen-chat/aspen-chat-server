use std::{collections::HashMap, pin::pin, sync::Arc, time::Duration};

use crate::app;
use crate::aspen_config::aspen_config;
use async_nats::{
    ConnectOptions, HeaderMap, Message, PublishError, Request, RequestError, ServerInfo,
    Statistics, Subject, SubscribeError, Subscriber,
    client::{DrainError, FlushError, ReconnectError},
    connection::State,
    subject::ToSubject,
};
use futures_util::StreamExt;
use tokio::sync::broadcast;

/// Centralizes NATS Subscribers preventing multiple backhaul connections.
/// Additionally tracks which of the active user connections are subscribed to each
/// NATS subject. If all interest in a particular NATS subject has vanished, the subscriber
/// is dropped. Otherwise, this is just a transparent wrapper around `async_nats::Client`
pub struct NatsConnectionManager {
    subscriptions: HashMap<String, broadcast::WeakSender<async_nats::Message>>,
    queue_subscriptions: HashMap<(String, String), broadcast::WeakSender<async_nats::Message>>,
    client: async_nats::Client,
}

impl NatsConnectionManager {
    pub async fn new(url: String, auth_token: String) -> Result<Self, app::Error> {
        let client =
            async_nats::connect_with_options(url, ConnectOptions::new().token(auth_token)).await?;
        Ok(Self {
            subscriptions: HashMap::default(),
            queue_subscriptions: HashMap::default(),
            client,
        })
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.client.timeout()
    }

    pub fn server_info(&self) -> ServerInfo {
        self.client.server_info()
    }

    pub fn is_server_compatible(&self, major: i64, minor: i64, patch: i64) -> bool {
        self.client.is_server_compatible(major, minor, patch)
    }

    pub async fn publish<S: ToSubject>(
        &self,
        subject: S,
        payload: bytes::Bytes,
    ) -> Result<(), PublishError> {
        self.client.publish(subject, payload).await
    }

    pub async fn publish_with_headers<S: ToSubject>(
        &self,
        subject: S,
        headers: HeaderMap,
        payload: bytes::Bytes,
    ) -> Result<(), PublishError> {
        self.client
            .publish_with_headers(subject, headers, payload)
            .await
    }

    pub async fn publish_with_reply<S: ToSubject, R: ToSubject>(
        &self,
        subject: S,
        reply: R,
        payload: bytes::Bytes,
    ) -> Result<(), PublishError> {
        self.client
            .publish_with_reply(subject, reply, payload)
            .await
    }

    pub async fn publish_with_reply_and_headers<S: ToSubject, R: ToSubject>(
        &self,
        subject: S,
        reply: R,
        headers: HeaderMap,
        payload: bytes::Bytes,
    ) -> Result<(), PublishError> {
        self.client
            .publish_with_reply_and_headers(subject, reply, headers, payload)
            .await
    }

    pub async fn request<S: ToSubject>(
        &self,
        subject: S,
        payload: bytes::Bytes,
    ) -> Result<Message, RequestError> {
        self.client.request(subject, payload).await
    }

    pub async fn request_with_headers<S: ToSubject>(
        &self,
        subject: S,
        headers: HeaderMap,
        payload: bytes::Bytes,
    ) -> Result<Message, RequestError> {
        self.client
            .request_with_headers(subject, headers, payload)
            .await
    }

    pub async fn send_request<S: ToSubject>(
        &self,
        subject: S,
        request: Request,
    ) -> Result<Message, RequestError> {
        self.client.send_request(subject, request).await
    }

    pub fn new_inbox(&self) -> String {
        self.client.new_inbox()
    }

    pub async fn subscribe<S: ToSubject>(
        &mut self,
        subject: S,
    ) -> Result<broadcast::Receiver<Message>, SubscribeError> {
        let subject = subject.to_subject();
        match self.subscriptions.get(subject.as_str()) {
            Some(sender) => match sender.upgrade() {
                Some(sender) => Ok(sender.subscribe()),
                None => self.new_backhaul_connection_subscribe(subject).await,
            },
            None => self.new_backhaul_connection_subscribe(subject).await,
        }
    }

    async fn new_backhaul_connection_subscribe(
        &mut self,
        subject: Subject,
    ) -> Result<broadcast::Receiver<Message>, SubscribeError> {
        let subscriber = self.client.subscribe(subject.clone()).await?;
        let config = aspen_config().await;
        let (sender, receiver) = broadcast::channel(config.event_queue_size);
        let weak_sender = sender.downgrade();
        self.subscriptions
            .insert(subject.into_string(), weak_sender);
        drive_broadcast_channel(sender, subscriber);
        Ok(receiver)
    }

    pub async fn queue_subscribe<S: ToSubject>(
        &mut self,
        subject: S,
        queue_group: String,
    ) -> Result<broadcast::Receiver<Message>, SubscribeError> {
        let subject = subject.to_subject();
        match self
            .queue_subscriptions
            .get(&(subject.to_string(), queue_group.clone()))
        {
            Some(sender) => match sender.upgrade() {
                Some(sender) => Ok(sender.subscribe()),
                None => {
                    self.new_backhaul_connection_queue_subscribe(subject, queue_group)
                        .await
                }
            },
            None => {
                self.new_backhaul_connection_queue_subscribe(subject, queue_group)
                    .await
            }
        }
    }

    async fn new_backhaul_connection_queue_subscribe(
        &mut self,
        subject: Subject,
        queue_group: String,
    ) -> Result<broadcast::Receiver<Message>, SubscribeError> {
        let subscriber = self
            .client
            .queue_subscribe(subject.clone(), queue_group.clone())
            .await?;
        let config = aspen_config().await;
        let (sender, receiver) = broadcast::channel(config.event_queue_size);
        let weak_sender = sender.downgrade();
        self.queue_subscriptions
            .insert((subject.into_string(), queue_group), weak_sender);
        drive_broadcast_channel(sender, subscriber);
        Ok(receiver)
    }

    pub async fn flush(&self) -> Result<(), FlushError> {
        self.client.flush().await
    }

    pub async fn drain(&self) -> Result<(), DrainError> {
        self.client.drain().await
    }

    pub fn connection_state(&self) -> State {
        self.client.connection_state()
    }

    pub async fn force_reconnect(&self) -> Result<(), ReconnectError> {
        self.client.force_reconnect().await
    }

    pub fn statistics(&self) -> Arc<Statistics> {
        self.client.statistics()
    }
}

fn drive_broadcast_channel(sender: broadcast::Sender<Message>, mut subscriber: Subscriber) {
    tokio::spawn(async move {
        let mut receivers_gone = pin!(sender.closed());
        loop {
            tokio::select! {
                _ = &mut receivers_gone => {
                    break;
                },
                msg = subscriber.next() => {
                    match msg {
                        Some(msg) => {
                            if let Err(_) = sender.send(msg) {
                                break;
                            }
                        }
                        None => break,
                    }
                },
            }
        }
    });
}
