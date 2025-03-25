use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use futures_util::{stream::SplitStream, SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use anyhow::{Result, Context};

pub struct WsClient {
    write: futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    read: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    active_subscriptions: Arc<Mutex<HashSet<String>>>,
}

impl WsClient {
    /// Creates a new WebSocket client for HyperLiquid
    pub async fn new(endpoint: &str) -> Result<Self> {
        let url = Url::parse(endpoint).context("Invalid WebSocket URL")?;
        let (ws_stream, _) = connect_async(url).await.context("Failed to connect to WebSocket")?;
        let (write, read) = ws_stream.split();
        Ok(Self { write, read, active_subscriptions: Arc::new(Mutex::new(HashSet::new())) })
    }

    /// Subscribes to a specific feed
    pub async fn subscribe(&mut self, subscription: Value) -> Result<()> {
        let sub_str = subscription.to_string();
        let mut active_subs = self.active_subscriptions.lock().await;

        if active_subs.contains(&sub_str) {
            println!("Already subscribed to: {}", sub_str);
            return Ok(());
        }

        let msg = json!({ "method": "subscribe", "subscription": subscription }).to_string();
        self.write.send(Message::Text(msg)).await.context("Failed to send subscription")?;

        active_subs.insert(sub_str);
        Ok(())
    }

    /// Unsubscribes from a specific feed
    pub async fn unsubscribe(&mut self, subscription: Value) -> Result<()> {
        let sub_str = subscription.to_string();
        let mut active_subs = self.active_subscriptions.lock().await;

        if !active_subs.contains(&sub_str) {
            println!("Not subscribed to: {}", sub_str);
            return Ok(());
        }

        let msg = json!({ "method": "unsubscribe", "subscription": subscription }).to_string();
        self.write.send(Message::Text(msg)).await.context("Failed to send unsubscription")?;

        active_subs.remove(&sub_str);
        Ok(())
    }

    /// Returns the list of active subscriptions
    pub async fn get_active_subscriptions(&self) -> Vec<String> {
        let active_subs = self.active_subscriptions.lock().await;
        active_subs.iter().cloned().collect()
    }

    /// Starts listening for incoming messages
    pub async fn listen<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(String) + Send + 'static,
    {
        while let Some(msg) = self.read.next().await {
            match msg {
                Ok(Message::Text(text)) => handler(text),
                Ok(_) => continue, // Ignore non-text messages
                Err(e) => eprintln!("WebSocket error: {e}"),
            }
        }
        Ok(())
    }
}