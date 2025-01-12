use anyhow::Context;
use serde::{Deserialize, Serialize};

pub mod announce;
pub mod scrape;

pub use announce::*;
pub use scrape::*;

/// Message received by tracker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InMessage {
    AnnounceRequest(AnnounceRequest),
    ScrapeRequest(ScrapeRequest),
}

#[cfg(feature = "tungstenite")]
impl InMessage {
    #[inline]
    pub fn to_ws_message(&self) -> ::tungstenite::Message {
        ::tungstenite::Message::from(::serde_json::to_string(&self).unwrap())
    }

    #[inline]
    pub fn from_ws_message(ws_message: tungstenite::Message) -> ::anyhow::Result<Self> {
        use tungstenite::Message;

        match ws_message {
            Message::Text(text) => {
                let mut text: Vec<u8> = text.as_bytes().to_owned();

                ::simd_json::serde::from_slice(&mut text).context("deserialize with serde")
            }
            Message::Binary(bytes) => {
                let mut bytes = bytes.to_vec();

                ::simd_json::serde::from_slice(&mut bytes[..]).context("deserialize with serde")
            }
            _ => Err(anyhow::anyhow!("Message is neither text nor binary")),
        }
    }
}
