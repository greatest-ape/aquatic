use serde::{Deserialize, Serialize};

pub mod announce;
pub mod answer;
pub mod error;
pub mod offer;
pub mod scrape;

pub use announce::*;
pub use answer::*;
pub use error::*;
pub use offer::*;
pub use scrape::*;

/// Message sent by tracker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutMessage {
    OfferOutMessage(OfferOutMessage),
    AnswerOutMessage(AnswerOutMessage),
    AnnounceResponse(AnnounceResponse),
    ScrapeResponse(ScrapeResponse),
    ErrorResponse(ErrorResponse),
}

#[cfg(feature = "tungstenite")]
impl OutMessage {
    #[inline]
    pub fn to_ws_message(&self) -> tungstenite::Message {
        ::tungstenite::Message::from(::serde_json::to_string(&self).unwrap())
    }

    #[inline]
    pub fn from_ws_message(message: ::tungstenite::Message) -> ::anyhow::Result<Self> {
        use tungstenite::Message::{Binary, Text};

        let mut text: Vec<u8> = match message {
            Text(text) => text.as_bytes().to_owned(),
            Binary(bytes) => String::from_utf8(bytes.to_vec())?.into(),
            _ => return Err(anyhow::anyhow!("Message is neither text nor bytes")),
        };

        Ok(::simd_json::serde::from_slice(&mut text)?)
    }
}
