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

impl InMessage {
    #[inline]
    pub fn to_ws_message(&self) -> ::tungstenite::Message {
        ::tungstenite::Message::from(::serde_json::to_string(&self).unwrap())
    }

    #[inline]
    pub fn from_ws_message(ws_message: tungstenite::Message) -> ::anyhow::Result<Self> {
        use tungstenite::Message::Text;

        let mut text = if let Text(text) = ws_message {
            text
        } else {
            return Err(anyhow::anyhow!("Message is not text"));
        };

        return ::simd_json::serde::from_str(&mut text).context("deserialize with serde");
    }
}
