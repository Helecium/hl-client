use std::collections::{HashSet, HashMap};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use futures_util::{stream::SplitStream, stream::SplitSink, SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use anyhow::{Result, Context};

pub struct WsClient {
    write: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    active_subscriptions: Arc<RwLock<HashSet<String>>>,
    channels: Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>,
}

impl WsClient {
    /// Initializes a new WebSocket client and establishes a connection to the specified endpoint.
    ///
    /// This function parses the provided WebSocket URL, connects to the WebSocket server, 
    /// and logs a message upon a successful connection. The WebSocket stream is then split 
    /// into a writer (`write`) and a reader (`read`). It also initializes storage for active 
    /// subscriptions and channel handlers.
    ///
    /// A separate asynchronous task is spawned to handle incoming WebSocket messages 
    /// continuously using the `listen_loop` function.

    pub async fn new(endpoint: &str) -> Result<Self> {
        let url = Url::parse(endpoint).context("Invalid WebSocket URL")?;
        let (ws_stream, _) = connect_async(url).await.context("Failed to connect to WebSocket")?;
        println!("WebSocket connected to {}", endpoint);  //Loggin message
        let (write, read) = ws_stream.split();

        let active_subscriptions = Arc::new(RwLock::new(HashSet::new()));
        let channels = Arc::new(RwLock::new(HashMap::new()));

        let channels_clone = channels.clone();
        tokio::spawn(async move {
            Self::listen_loop(read, channels_clone).await;
        });

        Ok(Self {
            write,
            active_subscriptions,
            channels,
        })
    }

    /// Continuously listens for incoming WebSocket messages and routes them to the appropriate channel.
    ///
    /// This function reads messages from the WebSocket stream in a loop. When a text message is received, 
    /// it attempts to parse it as JSON and extract the `channel` field. If a corresponding channel exists 
    /// in the `channels` map, the message is forwarded to its respective sender. 
    ///
    /// If the message type is unknown or an error occurs during message processing, 
    /// appropriate error handling is performed.
    async fn listen_loop(
        mut read_stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        channels: Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>,
    ) {
        while let Some(msg) = read_stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    //println!("Received message: {}", text);
                    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
                    if let Some(msg_type) = parsed["channel"].as_str() 
                    {
                        //println!("Message type: {}", msg_type);
                        let channels_read = channels.read().await;
                        if let Some(sender) = channels_read.get(msg_type) {
                            let _ = sender.send(text).await;
                        } else {
                            eprintln!("Unknown message type: {}", msg_type);
                        }
                    }
                }
                Ok(_) => continue,
                Err(e) => eprintln!("WebSocket error: {e}"),
            }
        }
    }

    /// Subscribes to a specific WebSocket feed and returns a receiver for incoming messages.
    ///
    /// This function sends a subscription request to the WebSocket server with the given subscription details. 
    /// If the subscription is new (i.e., not already active), it sends a subscription message to the server 
    /// and adds it to the active subscriptions list. Additionally, a new channel is created to receive messages 
    /// for this particular subscription type. The receiver (`rx`) is returned for further message processing.
    ///
    /// # Returns
    /// - A `mpsc::Receiver<String>` that allows receiving messages related to the subscription.
    pub async fn subscribe(&mut self, subscription: Value) -> Result<mpsc::Receiver<String>> {
        let sub_str = subscription.to_string();
        let msg_type = subscription["type"].as_str().unwrap_or_default().to_string();

        let mut active_subs = self.active_subscriptions.write().await;
        let mut channels = self.channels.write().await;

        if !active_subs.contains(&sub_str) {
            let msg = json!({ "method": "subscribe", "subscription": subscription }).to_string();
            //println!("Sending subscription: {}", msg);
            self.write.send(Message::Text(msg)).await.context("Failed to send subscription")?;
            active_subs.insert(sub_str);
        }

        let (tx, rx) = mpsc::channel(100);
        channels.insert(msg_type.clone(), tx);

        Ok(rx)
    }

    /// Unsubscribes from a specific WebSocket feed and removes the associated channel.
    ///
    /// This function sends an unsubscription request to the WebSocket server with the provided subscription details. 
    /// If the subscription is currently active, it removes it from the active subscriptions list and sends an 
    /// unsubscription message to the server. Additionally, it removes the associated channel from the `channels` map.
    ///
    /// # Returns
    /// - A `Result<()>` indicating success or failure of the unsubscription process.
    pub async fn unsubscribe(&mut self, subscription: Value) -> Result<()> {
        let sub_str = subscription.to_string();
        let msg_type = subscription["type"].as_str().unwrap_or_default().to_string();

        let mut active_subs = self.active_subscriptions.write().await;
        let mut channels = self.channels.write().await;

        if active_subs.contains(&sub_str) {
            let msg = json!({ "method": "unsubscribe", "subscription": subscription }).to_string();
            self.write.send(Message::Text(msg)).await.context("Failed to send unsubscription")?;
            active_subs.remove(&sub_str);
        }

        channels.remove(&msg_type);
        Ok(())
    }
}
