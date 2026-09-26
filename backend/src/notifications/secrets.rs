use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use rand::{RngCore, rng};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub target: String,
    pub signing_secret: Option<String>,
    pub token: Option<String>,
    pub bot_email: Option<String>,
    pub stream: Option<String>,
    pub topic: Option<String>,
}

impl ChannelConfig {
    pub fn encode(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn decode(value: &str) -> Result<Self> {
        Ok(serde_json::from_str(value)?)
    }
}

pub fn seal(key: &[u8], context: &str, plaintext: &str) -> Result<String> {
    if key.len() != 32 {
        bail!("encryption key must be 32 bytes");
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("invalid encryption key"))?;
    let mut nonce = [0_u8; 24];
    rng().fill_bytes(&mut nonce);
    let body = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            chacha20poly1305::aead::Payload {
                msg: plaintext.as_bytes(),
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("could not encrypt channel configuration"))?;
    let mut encoded = Vec::with_capacity(nonce.len() + body.len());
    encoded.extend_from_slice(&nonce);
    encoded.extend_from_slice(&body);
    Ok(BASE64.encode(encoded))
}

pub fn open(key: &[u8], context: &str, ciphertext: &str) -> Result<String> {
    let encoded = BASE64.decode(ciphertext)?;
    if key.len() != 32 || encoded.len() < 24 {
        bail!("encrypted configuration is invalid");
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("invalid encryption key"))?;
    let (nonce, body) = encoded.split_at(24);
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            chacha20poly1305::aead::Payload {
                msg: body,
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("encrypted configuration authentication failed"))?;
    Ok(String::from_utf8(plaintext)?)
}
