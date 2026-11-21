//! Encrypts a plugin session end to end with `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s` (ADR 0007),
//! so that nothing between the plugin and the bridge — the LAN, a tunnel or the relay — can read
//! or forge messages (Go: internal/securechan).
//!
//! Both sides derive the pre-shared key from the pairing token, so completing the handshake is
//! itself the authentication; every session still gets fresh keys from ephemeral DH.
//!
//! Framing: every WebSocket frame is binary. The first frame from the plugin is Noise message 1,
//! the bridge answers with message 2, and every later frame is one encrypted chunk whose first
//! plaintext byte says whether more chunks of the same message follow (Noise caps a single
//! ciphertext at 65535 bytes, and stroke data can be larger).
//!
//! This module is only the cryptography and the framing, with no I/O: the caller moves the byte
//! strings over its WebSocket. Frames must go onto the wire in the order `seal` made them (the
//! nonce counts them), so a caller with several writers enqueues them from inside `seal`.

use sha2::{Digest, Sha256};
use std::sync::Mutex;

const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"spellink/2";
const MAX_NOISE_MSG: usize = 65535;
const TAG: usize = 16;
/// Largest plaintext chunk: minus the AEAD tag and the continuation byte.
pub const MAX_PLAIN_CHUNK: usize = MAX_NOISE_MSG - TAG - 1;
/// Reassembled message limit.
pub const MAX_MESSAGE: usize = 4 << 20;
const MORE_FOLLOWS: u8 = 1;
const LAST_CHUNK: u8 = 0;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The handshake failed: with NNpsk0 a wrong key shows up on message 1.
    Handshake,
    /// A chunk did not decrypt: tampered, replayed, reordered or from another session.
    Decrypt,
    /// A decrypted chunk was empty, or the message grew past the limit.
    BadChunk,
    /// The cipher refused (nonce exhausted).
    Cipher,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::Handshake => "handshake failed",
            Error::Decrypt => "decrypt failed",
            Error::BadChunk => "bad chunk",
            Error::Cipher => "cipher error",
        })
    }
}

impl std::error::Error for Error {}

/// Derives the 32-byte pre-shared key from the pairing token.
pub fn psk(token: &str) -> [u8; 32] {
    Sha256::digest(format!("spellink-psk-v1:{token}").as_bytes()).into()
}

fn builder(psk: &[u8; 32]) -> snow::Builder<'_> {
    snow::Builder::new(PATTERN.parse().expect("the pattern is a constant")).prologue(PROLOGUE).expect("a prologue is accepted before the handshake").psk(0, psk).expect("a 32-byte key at position 0")
}

/// An established session's keys. `seal` and `open` may be called from different tasks.
pub struct Channel {
    state: Mutex<snow::TransportState>,
}

/// The plugin's side of a handshake in progress (the bridge's side is `Channel::accept`).
pub struct Initiator(snow::HandshakeState);

impl Initiator {
    /// Starts the handshake; the returned bytes are message 1, to send as one binary frame.
    pub fn start(psk: &[u8; 32]) -> Result<(Initiator, Vec<u8>), Error> {
        let mut hs = builder(psk).build_initiator().map_err(|_| Error::Handshake)?;
        let mut buf = vec![0u8; MAX_NOISE_MSG];
        let n = hs.write_message(&[], &mut buf).map_err(|_| Error::Handshake)?;
        buf.truncate(n);
        Ok((Initiator(hs), buf))
    }

    /// Completes the handshake with the bridge's answer (message 2).
    pub fn finish(mut self, msg2: &[u8]) -> Result<Channel, Error> {
        let mut buf = vec![0u8; MAX_NOISE_MSG];
        self.0.read_message(msg2, &mut buf).map_err(|_| Error::Handshake)?;
        Channel::from_handshake(self.0)
    }
}

impl Channel {
    fn from_handshake(hs: snow::HandshakeState) -> Result<Channel, Error> {
        if !hs.is_handshake_finished() {
            return Err(Error::Handshake);
        }
        let state = hs.into_transport_mode().map_err(|_| Error::Handshake)?;
        Ok(Channel { state: Mutex::new(state) })
    }

    /// Completes the handshake as the bridge, given the plugin's first frame (message 1). Returns
    /// the channel and message 2, to send as one binary frame. A wrong key fails here, before any
    /// application data is exchanged.
    pub fn accept(psk: &[u8; 32], msg1: &[u8]) -> Result<(Channel, Vec<u8>), Error> {
        let mut hs = builder(psk).build_responder().map_err(|_| Error::Handshake)?;
        let mut scratch = vec![0u8; MAX_NOISE_MSG];
        hs.read_message(msg1, &mut scratch).map_err(|_| Error::Handshake)?;
        let mut msg2 = vec![0u8; MAX_NOISE_MSG];
        let n = hs.write_message(&[], &mut msg2).map_err(|_| Error::Handshake)?;
        msg2.truncate(n);
        Ok((Channel::from_handshake(hs)?, msg2))
    }

    /// Encrypts one message as one or more chunks, handing each to `emit` in wire order while the
    /// session's send state is locked, so concurrent senders cannot interleave their chunks.
    /// `emit` must not block: enqueue to the writer.
    pub fn seal(&self, msg: &[u8], mut emit: impl FnMut(Vec<u8>) -> Result<(), Error>) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        let mut rest = msg;
        loop {
            let n = rest.len().min(MAX_PLAIN_CHUNK);
            let last = n == rest.len();
            let mut plain = Vec::with_capacity(n + 1);
            plain.push(if last { LAST_CHUNK } else { MORE_FOLLOWS });
            plain.extend_from_slice(&rest[..n]);
            let mut ct = vec![0u8; plain.len() + TAG];
            let len = state.write_message(&plain, &mut ct).map_err(|_| Error::Cipher)?;
            ct.truncate(len);
            emit(ct)?;
            rest = &rest[n..];
            if last {
                return Ok(());
            }
        }
    }

    /// Decrypts one received frame into `into` (a message being reassembled). Returns true when
    /// the message is complete. Any tampering fails decryption.
    pub fn open(&self, ct: &[u8], into: &mut Vec<u8>) -> Result<bool, Error> {
        let mut plain = vec![0u8; ct.len().max(TAG)];
        let n = self.state.lock().unwrap().read_message(ct, &mut plain).map_err(|_| Error::Decrypt)?;
        if n == 0 || into.len() + n > MAX_MESSAGE {
            return Err(Error::BadChunk);
        }
        into.extend_from_slice(&plain[1..n]);
        Ok(plain[0] == LAST_CHUNK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(token: &str) -> (Channel, Channel) {
        let (init, m1) = Initiator::start(&psk(token)).unwrap();
        let (server, m2) = Channel::accept(&psk(token), &m1).unwrap();
        (init.finish(&m2).unwrap(), server)
    }

    fn send(from: &Channel, msg: &[u8]) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        from.seal(msg, |f| {
            frames.push(f);
            Ok(())
        })
        .unwrap();
        frames
    }

    fn receive(to: &Channel, frames: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
        let mut msg = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            if to.open(f, &mut msg)? {
                assert_eq!(i, frames.len() - 1, "a message ends with its last chunk");
                return Ok(msg);
            }
        }
        panic!("the message never ended");
    }

    #[test]
    fn round_trip_including_chunked_messages_both_ways() {
        let (plugin, bridge) = pair("token");
        let big = "ストローク".repeat(30000).into_bytes(); // ~450 KB: many Noise chunks
        for msg in [br#"{"type":"hello"}"#.to_vec(), big, vec![]] {
            let frames = send(&plugin, &msg);
            assert_eq!(receive(&bridge, &frames).unwrap(), msg);
            let frames = send(&bridge, &msg);
            assert_eq!(receive(&plugin, &frames).unwrap(), msg);
        }
        assert_eq!(send(&plugin, &vec![7u8; 3 * MAX_PLAIN_CHUNK]).len(), 3, "exactly 3 full chunks: the third is the last, as in the Go bridge");
        assert_eq!(send(&plugin, &vec![7u8; 3 * MAX_PLAIN_CHUNK + 1]).len(), 4);
        assert_eq!(send(&plugin, &[]).len(), 1, "an empty message is one chunk");
    }

    #[test]
    fn a_wrong_key_is_rejected_on_the_first_message() {
        let (_init, m1) = Initiator::start(&psk("other")).unwrap();
        assert!(matches!(Channel::accept(&psk("token"), &m1), Err(Error::Handshake)));
    }

    #[test]
    fn a_garbage_first_frame_is_rejected() {
        assert!(Channel::accept(&psk("token"), b"hello").is_err());
        assert!(Channel::accept(&psk("token"), &[]).is_err());
    }

    #[test]
    fn a_wrong_answer_does_not_finish_the_handshake() {
        let (init, _m1) = Initiator::start(&psk("token")).unwrap();
        assert!(init.finish(&[0u8; 48]).is_err());
    }

    #[test]
    fn tampering_replaying_and_reordering_fail_decryption() {
        let (plugin, bridge) = pair("token");
        let a = send(&plugin, b"one").remove(0);
        let b = send(&plugin, b"two").remove(0);
        let mut flipped = a.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert_eq!(bridge.open(&flipped, &mut Vec::new()), Err(Error::Decrypt), "a flipped bit");
        assert_eq!(bridge.open(&b, &mut Vec::new()), Err(Error::Decrypt), "out of order: the nonce is wrong");
        let mut msg = Vec::new();
        assert_eq!(bridge.open(&a, &mut msg), Ok(true), "the real first frame still works after failures");
        assert_eq!(bridge.open(&a, &mut Vec::new()), Err(Error::Decrypt), "a replay");
        assert_eq!(bridge.open(&b, &mut Vec::new()), Ok(true));
    }

    #[test]
    fn frames_from_another_session_do_not_decrypt() {
        let (plugin, _) = pair("token");
        let (_, other_bridge) = pair("token");
        let f = send(&plugin, b"x").remove(0);
        assert_eq!(other_bridge.open(&f, &mut Vec::new()), Err(Error::Decrypt));
    }

    #[test]
    fn an_endless_message_is_cut_at_the_limit() {
        let (plugin, bridge) = pair("token");
        let chunk = vec![1u8; MAX_PLAIN_CHUNK];
        let mut into = Vec::new();
        let mut result = Ok(false);
        for _ in 0..(MAX_MESSAGE / MAX_PLAIN_CHUNK + 2) {
            // a hostile peer that never says "last"
            let mut plain = vec![MORE_FOLLOWS];
            plain.extend_from_slice(&chunk);
            let mut ct = vec![0u8; plain.len() + TAG];
            let n = plugin.state.lock().unwrap().write_message(&plain, &mut ct).unwrap();
            result = bridge.open(&ct[..n], &mut into);
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(Error::BadChunk));
        assert!(into.len() <= MAX_MESSAGE);
    }

    #[test]
    fn an_empty_chunk_is_a_bad_chunk() {
        let (plugin, bridge) = pair("token");
        let mut ct = vec![0u8; TAG];
        let n = plugin.state.lock().unwrap().write_message(&[], &mut ct).unwrap();
        assert_eq!(bridge.open(&ct[..n], &mut Vec::new()), Err(Error::BadChunk));
    }

    #[test]
    fn emit_failing_stops_sealing() {
        let (plugin, _) = pair("token");
        let mut n = 0;
        let r = plugin.seal(&vec![0u8; 5 * MAX_PLAIN_CHUNK], |_| {
            n += 1;
            if n == 2 { Err(Error::Cipher) } else { Ok(()) }
        });
        assert_eq!((r, n), (Err(Error::Cipher), 2));
    }

    #[test]
    fn the_key_is_derived_from_the_token_like_the_go_bridge() {
        assert_eq!(hex::encode(psk("token")), "3fd003afc2fa6ddb5b933b28569c8fb0a1e42eaea44a95023c65277738e15ee7");
        assert_ne!(psk("a"), psk("b"));
    }
}
