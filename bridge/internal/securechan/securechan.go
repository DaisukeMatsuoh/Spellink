// Package securechan encrypts a plugin session end to end with
// Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s (ADR 0007), so that nothing between the plugin and the
// bridge — the LAN, a tunnel or the relay — can read or forge messages.
//
// Both sides derive the pre-shared key from the pairing token, so completing the handshake is
// itself the authentication; every session still gets fresh keys from ephemeral DH.
//
// Framing: every WebSocket frame is binary. The first frame from the plugin is Noise message 1,
// the bridge answers with message 2, and every later frame is one encrypted chunk whose first
// plaintext byte says whether more chunks of the same message follow (Noise caps a single
// ciphertext at 65535 bytes, and stroke data can be larger).
package securechan

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"sync"

	"github.com/coder/websocket"
	"github.com/flynn/noise"
)

const (
	maxPlainChunk = noise.MaxMsgLen - 16 - 1 // minus the AEAD tag and the continuation byte
	maxMessage    = 4 << 20                  // reassembled message limit
	moreFollows   = 1
	lastChunk     = 0
)

var (
	prologue = []byte("spellink/2")
	suite    = noise.NewCipherSuite(noise.DH25519, noise.CipherChaChaPoly, noise.HashBLAKE2s)
)

// PSK derives the 32-byte pre-shared key from the pairing token.
func PSK(token string) []byte {
	h := sha256.Sum256([]byte("spellink-psk-v1:" + token))
	return h[:]
}

// Channel is an established encrypted session over a WebSocket.
type Channel struct {
	c      *websocket.Conn
	sendMu sync.Mutex
	send   *noise.CipherState
	recv   *noise.CipherState
}

func config(psk []byte, initiator bool) noise.Config {
	return noise.Config{
		CipherSuite: suite, Pattern: noise.HandshakeNN, Initiator: initiator,
		Prologue: prologue, PresharedKey: psk, PresharedKeyPlacement: 0,
	}
}

// Accept completes the handshake as the bridge, given the plugin's first frame (message 1).
// A wrong key fails here, before any application data is exchanged.
func Accept(ctx context.Context, c *websocket.Conn, psk, msg1 []byte) (*Channel, error) {
	hs, err := noise.NewHandshakeState(config(psk, false))
	if err != nil {
		return nil, err
	}
	if _, _, _, err := hs.ReadMessage(nil, msg1); err != nil {
		return nil, fmt.Errorf("handshake: %w", err)
	}
	msg2, toInitiator, toResponder, err := hs.WriteMessage(nil, nil)
	if err != nil {
		return nil, err
	}
	if err := c.Write(ctx, websocket.MessageBinary, msg2); err != nil {
		return nil, err
	}
	// flynn/noise returns (initiator→responder, responder→initiator) after the final message.
	return &Channel{c: c, send: toResponder, recv: toInitiator}, nil
}

// Dial performs the handshake as the plugin (used by tests and Go clients).
func Dial(ctx context.Context, c *websocket.Conn, psk []byte) (*Channel, error) {
	hs, err := noise.NewHandshakeState(config(psk, true))
	if err != nil {
		return nil, err
	}
	msg1, _, _, err := hs.WriteMessage(nil, nil)
	if err != nil {
		return nil, err
	}
	if err := c.Write(ctx, websocket.MessageBinary, msg1); err != nil {
		return nil, err
	}
	typ, msg2, err := c.Read(ctx)
	if err != nil {
		return nil, err
	}
	if typ != websocket.MessageBinary {
		return nil, errors.New("handshake: expected a binary frame")
	}
	_, toResponder, toInitiator, err := hs.ReadMessage(nil, msg2)
	if err != nil {
		return nil, fmt.Errorf("handshake: %w", err)
	}
	return &Channel{c: c, send: toResponder, recv: toInitiator}, nil
}

// Write encrypts and sends one message, split into chunks as needed.
func (ch *Channel) Write(ctx context.Context, msg []byte) error {
	ch.sendMu.Lock()
	defer ch.sendMu.Unlock()
	for {
		n := min(len(msg), maxPlainChunk)
		flag := byte(lastChunk)
		if n < len(msg) {
			flag = moreFollows
		}
		plain := append([]byte{flag}, msg[:n]...)
		ct, err := ch.send.Encrypt(nil, nil, plain)
		if err != nil {
			return err
		}
		if err := ch.c.Write(ctx, websocket.MessageBinary, ct); err != nil {
			return err
		}
		msg = msg[n:]
		if flag == lastChunk {
			return nil
		}
	}
}

// Read receives and decrypts one whole message. Any tampering fails decryption.
func (ch *Channel) Read(ctx context.Context) ([]byte, error) {
	var msg []byte
	for {
		typ, ct, err := ch.c.Read(ctx)
		if err != nil {
			return nil, err
		}
		if typ != websocket.MessageBinary {
			return nil, errors.New("expected a binary frame")
		}
		plain, err := ch.recv.Decrypt(nil, nil, ct)
		if err != nil {
			return nil, fmt.Errorf("decrypt: %w", err)
		}
		if len(plain) == 0 || len(msg)+len(plain) > maxMessage {
			return nil, errors.New("bad chunk")
		}
		msg = append(msg, plain[1:]...)
		if plain[0] == lastChunk {
			return msg, nil
		}
	}
}
