use std::fmt::Debug;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{anyhow, bail, Result};
use bytes::{Bytes, BytesMut};
use log::warn;
use tokio_stream::Stream;

use crate::ts;

const INITIAL_BUFFER: usize = 4096;

#[derive(Debug)]
enum State {
    Initial,
    Buffering,
    Closed,
}

#[derive(Debug)]
pub struct Buffer<S> {
    inner: S,
    state: State,
    counter: u8,
    buf: BytesMut,
}

impl<S> Buffer<S> {
    pub fn new(stream: S) -> Self {
        Buffer {
            inner: stream,
            state: State::Initial,
            counter: 0,
            buf: BytesMut::with_capacity(INITIAL_BUFFER),
        }
    }

    fn get_bytes(&mut self) -> Result<Bytes> {
        if self.buf.len() < 6 {
            bail!("not enough data");
        }
        let pes_packet_length = (usize::from(self.buf[4]) << 8) | usize::from(self.buf[5]);
        if pes_packet_length == 0 {
            return Ok(self.buf.split().freeze());
        }
        if self.buf.len() < pes_packet_length + 6 {
            bail!(
                "not enough data. needs: {}, has: {}",
                pes_packet_length + 6,
                self.buf.len()
            );
        }
        return Ok(self.buf.split_to(pes_packet_length + 6).freeze());
    }
}

impl<S> Stream for Buffer<S>
where
    S: Stream<Item = ts::TSPacket> + Unpin,
{
    type Item = Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let State::Closed = self.state {
                return Poll::Ready(None);
            }

            let packet = match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(packet)) => packet,
                Poll::Ready(None) => {
                    let old_state = mem::replace(&mut self.state, State::Closed);
                    if let State::Buffering = old_state {
                        return Poll::Ready(Some(self.get_bytes()));
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            };

            if packet.transport_error_indicator {
                continue;
            }

            let data = match packet.data {
                Some(ref data) => data.as_ref(),
                None => return Poll::Ready(Some(Err(anyhow!("no data")))),
            };

            if packet.payload_unit_start_indicator {
                let mut bytes = None;
                if let State::Buffering = self.state {
                    bytes = Some(self.get_bytes());
                }

                self.state = State::Buffering;
                self.counter = packet.continuity_counter;
                self.buf.clear();
                self.buf.extend_from_slice(data);

                return match bytes {
                    Some(Ok(bytes)) => Poll::Ready(Some(Ok(bytes))),
                    Some(Err(e)) => {
                        warn!("an error happened, ignore: {:?}", e);
                        continue;
                    }
                    None => continue,
                };
            } else {
                if let State::Initial = self.state {
                    // seen partial packet
                    continue;
                }

                if self.counter == packet.continuity_counter {
                    // duplicate packet
                    continue;
                } else if (self.counter + 1) % 16 == packet.continuity_counter {
                    self.counter = packet.continuity_counter;
                } else {
                    self.state = State::Initial;
                    self.buf.clear();
                    return Poll::Ready(Some(Err(anyhow!("pes packet discontinued"))));
                }

                self.buf.extend_from_slice(data);
            }
        }
    }
}
