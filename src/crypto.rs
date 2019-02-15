// VpnCloud - Peer-to-Peer VPN
// Copyright (C) 2015-2019  Dennis Schwerdel
// This software is licensed under GPL-3 or newer (see LICENSE.md)

use std::num::NonZeroU32;

use ring::aead::*;
use ring::pbkdf2;
use ring::rand::*;
use ring::digest::*;

use super::types::Error;


const SALT: &[u8; 32] = b"vpncloudVPNCLOUDvpncl0udVpnCloud";
const HEX_PREFIX: &str = "hex:";
const HASH_PREFIX: &str = "hash:";

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
pub enum CryptoMethod {
    #[serde(rename = "chacha20")]
    ChaCha20,
    #[serde(rename = "aes256")]
    AES256
}

pub struct CryptoData {
    sealing_key: SealingKey,
    opening_key: OpeningKey,
    nonce: Vec<u8>
}

#[allow(unknown_lints, clippy::large_enum_variant)]
pub enum Crypto {
    None,
    ChaCha20Poly1305(CryptoData),
    AES256GCM(CryptoData)
}

fn inc_nonce(nonce: &mut [u8]) {
    let l = nonce.len();
    for i in (0..l).rev() {
        let mut num = nonce[i];
        num = num.wrapping_add(1);
        nonce[i] = num;
        if num > 0 {
            return
        }
    }
    warn!("Nonce overflowed");
}


impl Crypto {
    #[inline]
    pub fn init() {
    }

    #[inline]
    pub fn sodium_version() -> String {
        "0".to_string()
    }

    #[inline]
    pub fn aes256_available() -> bool {
        true
    }

    #[inline]
    pub fn method(&self) -> u8 {
        match *self {
            Crypto::None => 0,
            Crypto::ChaCha20Poly1305{..} => 1,
            Crypto::AES256GCM{..} => 2
        }
    }

    #[inline]
    pub fn nonce_bytes(&self) -> usize {
        match *self {
            Crypto::None => 0,
            Crypto::ChaCha20Poly1305(ref data) | Crypto::AES256GCM(ref data) => data.sealing_key.algorithm().nonce_len()
        }
    }

    #[inline]
    #[allow(unknown_lints,clippy::match_same_arms)]
    pub fn additional_bytes(&self) -> usize {
        match *self {
            Crypto::None => 0,
            Crypto::ChaCha20Poly1305(ref data) | Crypto::AES256GCM(ref data) => data.sealing_key.algorithm().tag_len()
        }
    }

    pub fn from_shared_key(method: CryptoMethod, password: &str) -> Self {
        let algo = match method {
            CryptoMethod::ChaCha20 => &CHACHA20_POLY1305,
            CryptoMethod::AES256 => &AES_256_GCM
        };
        let mut key: Vec<u8> = Vec::with_capacity(algo.key_len());
        for _ in 0..algo.key_len() {
            key.push(0);
        }
        if password.starts_with(HEX_PREFIX) {
            let password = &password[HEX_PREFIX.len()..];
            if password.len() != 2 * algo.key_len() {
                fail!("Raw secret key must be exactly {} bytes long", algo.key_len());
            }
            for i in 0..algo.key_len() {
                key[i] = try_fail!(u8::from_str_radix(&password[2*i..2*i+1], 16), "Failed to parse raw secret key: {}");
            }
        } else {
            let password = if password.starts_with(HASH_PREFIX) {
                &password[HASH_PREFIX.len()..]
            } else {
                password
            };
            pbkdf2::derive(&SHA256, NonZeroU32::new(4096).unwrap(), SALT, password.as_bytes(), &mut key);
        }
        let sealing_key = SealingKey::new(algo, &key[..algo.key_len()]).expect("Failed to create key");
        let opening_key = OpeningKey::new(algo, &key[..algo.key_len()]).expect("Failed to create key");
        let mut nonce: Vec<u8> = Vec::with_capacity(algo.nonce_len());
        for _ in 0..algo.nonce_len() {
            nonce.push(0);
        }
        // leave the highest byte of the nonce 0 so it will not overflow
        if SystemRandom::new().fill(&mut nonce[1..]).is_err() {
            fail!("Randomizing nonce failed");
        }
        let data = CryptoData { sealing_key, opening_key, nonce };
        match method {
            CryptoMethod::ChaCha20 => Crypto::ChaCha20Poly1305(data),
            CryptoMethod::AES256 => Crypto::AES256GCM(data)
        }
    }

    pub fn decrypt(&self, buf: &mut [u8], nonce: &[u8], header: &[u8]) -> Result<usize, Error> {
        match *self {
            Crypto::None => Ok(buf.len()),
            Crypto::ChaCha20Poly1305(ref data) | Crypto::AES256GCM(ref data) => {
                let nonce = Nonce::try_assume_unique_for_key(nonce).unwrap();
                match open_in_place(&data.opening_key, nonce, Aad::from(header), 0, buf) {
                   Ok(plaintext) => Ok(plaintext.len()),
                   Err(_) => Err(Error::Crypto("Failed to decrypt"))
                }
            }
        }
    }

    pub fn encrypt(&mut self, buf: &mut [u8], mlen: usize, nonce_bytes: &mut [u8], header: &[u8]) -> usize {
        let tag_len = self.additional_bytes();
        match *self {
            Crypto::None => mlen,
            Crypto::ChaCha20Poly1305(ref mut data) | Crypto::AES256GCM(ref mut data) => {
                inc_nonce(&mut data.nonce);
                assert!(buf.len() - mlen >= tag_len);
                let buf = &mut buf[.. mlen + tag_len];
                let nonce = Nonce::try_assume_unique_for_key(&data.nonce).unwrap();
                let new_len = seal_in_place(&data.sealing_key, nonce, Aad::from(header), buf, tag_len).expect("Failed to encrypt");
                nonce_bytes.clone_from_slice(&data.nonce);
                new_len
            }
        }
    }
}
