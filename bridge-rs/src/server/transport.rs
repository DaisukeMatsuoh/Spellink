//! A WebSocket as the session sees it: a stream of incoming messages and a queue of outgoing
//! ones. A task owns the real socket's write side and drains the queue in order; the session
//! never waits on the network when it sends (a slow or dead peer shows up as a full or closed
//! queue, which a run turns into SEND_FAIL).

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::pin::Pin;
use tokio::sync::mpsc;

/// What the peer sent.
#[derive(Debug, Clone, PartialEq)]
pub enum WsMsg {
    Text(String),
    Binary(Vec<u8>),
}

/// What we send. `Close` ends the connection with a code and a reason, after what was queued.
#[derive(Debug, Clone, PartialEq)]
pub enum Wire {
    Text(String),
    Binary(Vec<u8>),
    Close(u16, String),
}

/// Incoming messages end with `None` when the peer closed (or the connection broke); an `Err` is a
/// protocol error (a frame that is too large, a bad close).
pub type Incoming = Pin<Box<dyn Stream<Item = Result<WsMsg, String>> + Send>>;

/// How many outgoing messages may wait: an answer is a few hundred frames. A peer that is not
/// reading, with this many waiting, is treated as gone.
pub const QUEUE: usize = 4096;

pub struct Transport {
    pub rx: Incoming,
    pub tx: mpsc::Sender<Wire>,
}

/// Runs the writer: queue → socket. Ends on `Close`, on a write error, or when every sender is gone.
fn writer<S, M, E>(mut sink: S, mut q: mpsc::Receiver<Wire>, conv: impl Fn(Wire) -> M + Send + 'static, is_close: impl Fn(&Wire) -> bool + Send + 'static)
where
    S: Sink<M, Error = E> + Unpin + Send + 'static,
    M: Send + 'static,
    E: Send + 'static,
{
    tokio::spawn(async move {
        while let Some(w) = q.recv().await {
            let close = is_close(&w);
            if sink.send(conv(w)).await.is_err() || close {
                break;
            }
        }
        let _ = sink.close().await;
    });
}

impl Transport {
    pub fn from_axum(ws: axum::extract::ws::WebSocket) -> Transport {
        use axum::extract::ws::{CloseFrame, Message};
        let (sink, stream) = ws.split();
        let (tx, q) = mpsc::channel(QUEUE);
        writer(
            sink,
            q,
            |w| match w {
                Wire::Text(t) => Message::Text(t.into()),
                Wire::Binary(b) => Message::Binary(b.into()),
                Wire::Close(code, reason) => Message::Close(Some(CloseFrame { code, reason: reason.into() })),
            },
            |w| matches!(w, Wire::Close(..)),
        );
        let rx = stream.filter_map(|m| async move {
            match m {
                Ok(Message::Text(t)) => Some(Ok(WsMsg::Text(t.to_string()))),
                Ok(Message::Binary(b)) => Some(Ok(WsMsg::Binary(b.to_vec()))),
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => None,
                Ok(Message::Close(_)) => Some(Err("closed".to_string())),
                Err(e) => Some(Err(e.to_string())),
            }
        });
        Transport { rx: Box::pin(end_on_close(rx)), tx }
    }

    pub fn from_tungstenite<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> Transport
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;
        use tokio_tungstenite::tungstenite::Message;
        let (sink, stream) = ws.split();
        let (tx, q) = mpsc::channel(QUEUE);
        writer(
            sink,
            q,
            |w| match w {
                Wire::Text(t) => Message::Text(t.into()),
                Wire::Binary(b) => Message::Binary(b.into()),
                Wire::Close(code, reason) => Message::Close(Some(CloseFrame { code: CloseCode::from(code), reason: reason.into() })),
            },
            |w| matches!(w, Wire::Close(..)),
        );
        let rx = stream.filter_map(|m| async move {
            match m {
                Ok(Message::Text(t)) => Some(Ok(WsMsg::Text(t.to_string()))),
                Ok(Message::Binary(b)) => Some(Ok(WsMsg::Binary(b.to_vec()))),
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => None,
                Ok(Message::Close(_)) => Some(Err("closed".to_string())),
                Err(e) => Some(Err(e.to_string())),
            }
        });
        Transport { rx: Box::pin(end_on_close(rx)), tx }
    }

    /// An in-memory pair for tests: what one side sends, the other receives.
    pub fn memory_pair() -> (Transport, Transport) {
        let (a_tx, mut a_q) = mpsc::channel::<Wire>(QUEUE);
        let (b_tx, mut b_q) = mpsc::channel::<Wire>(QUEUE);
        let (a_in_tx, a_in) = mpsc::unbounded_channel::<Result<WsMsg, String>>();
        let (b_in_tx, b_in) = mpsc::unbounded_channel::<Result<WsMsg, String>>();
        fn pump(mut q: mpsc::Receiver<Wire>, to: mpsc::UnboundedSender<Result<WsMsg, String>>) {
            tokio::spawn(async move {
                while let Some(w) = q.recv().await {
                    match w {
                        Wire::Text(t) => drop(to.send(Ok(WsMsg::Text(t)))),
                        Wire::Binary(b) => drop(to.send(Ok(WsMsg::Binary(b)))),
                        Wire::Close(code, reason) => {
                            let _ = to.send(Err(format!("closed {code} {reason}")));
                            break;
                        }
                    }
                }
            });
        }
        pump(std::mem::replace(&mut a_q, mpsc::channel(1).1), b_in_tx);
        pump(std::mem::replace(&mut b_q, mpsc::channel(1).1), a_in_tx);
        let stream = |rx: mpsc::UnboundedReceiver<Result<WsMsg, String>>| -> Incoming { Box::pin(tokio_stream_from(rx)) };
        (Transport { rx: stream(a_in), tx: a_tx }, Transport { rx: stream(b_in), tx: b_tx })
    }
}

fn tokio_stream_from(mut rx: mpsc::UnboundedReceiver<Result<WsMsg, String>>) -> impl Stream<Item = Result<WsMsg, String>> + Send {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

/// After an error (the close, a broken frame) nothing more is read.
fn end_on_close<S: Stream<Item = Result<WsMsg, String>>>(s: S) -> impl Stream<Item = Result<WsMsg, String>> {
    let mut done = false;
    s.take_while(move |m| {
        let stop = done;
        if m.is_err() {
            done = true;
        }
        std::future::ready(!stop)
    })
}
