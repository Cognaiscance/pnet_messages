//! App payloads. pNet carries them and does not read them.
//!
//! Each payload starts with a version byte and a type byte. Text is at most
//! [`MAX_TEXT`] bytes so one message stays inside a single datagram.

use crate::store::Message;

pub const VERSION: u8 = 1;
pub const MAX_TEXT: usize = 800;

pub const DELIVER: u8 = 0x03;
pub const DELIVER_ACK: u8 = 0x04;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Deliver(Message),
    DeliverAck { id: [u8; 16] },
}

pub fn encode(frame: &Frame) -> Option<Vec<u8>> {
    let mut buf = vec![VERSION];
    match frame {
        Frame::Deliver(message) => {
            if message.text.len() > MAX_TEXT || !is_text(&message.text) {
                return None;
            }
            buf.push(DELIVER);
            push_message(&mut buf, message);
        }
        Frame::DeliverAck { id } => {
            buf.push(DELIVER_ACK);
            buf.extend_from_slice(id);
        }
    }
    Some(buf)
}

pub fn decode(bytes: &[u8]) -> Option<Frame> {
    if bytes.first().copied()? != VERSION || bytes.len() < 2 {
        return None;
    }
    let mut cur = Cursor { data: bytes, pos: 2 };
    match bytes[1] {
        DELIVER => Some(Frame::Deliver(cur.message()?)),
        DELIVER_ACK => Some(Frame::DeliverAck { id: cur.array()? }),
        _ => None,
    }
}

pub fn is_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_TEXT && !text.contains('\0')
}

fn push_text(buf: &mut Vec<u8>, text: &str) {
    let len = u16::try_from(text.len()).unwrap_or(u16::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(text.as_bytes());
}

fn push_message(buf: &mut Vec<u8>, message: &Message) {
    buf.extend_from_slice(&message.id);
    buf.extend_from_slice(&message.from_user);
    buf.extend_from_slice(&message.to_user);
    buf.extend_from_slice(&message.at_ms.to_be_bytes());
    push_text(buf, &message.text);
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u64(&mut self) -> Option<u64> {
        let bytes: [u8; 8] = self.array()?;
        Some(u64::from_be_bytes(bytes))
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.pos.checked_add(N)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        slice.try_into().ok()
    }

    fn text(&mut self) -> Option<String> {
        let len_bytes: [u8; 2] = self.array()?;
        let len = u16::from_be_bytes(len_bytes) as usize;
        if len > MAX_TEXT {
            return None;
        }
        let end = self.pos.checked_add(len)?;
        let slice = self.data.get(self.pos..end)?;
        let text = std::str::from_utf8(slice).ok()?.to_string();
        if !is_text(&text) {
            return None;
        }
        self.pos = end;
        Some(text)
    }

    fn message(&mut self) -> Option<Message> {
        Some(Message {
            id: self.array()?,
            from_user: self.array()?,
            to_user: self.array()?,
            at_ms: self.u64()?,
            text: self.text()?,
            delivered: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Message {
        Message {
            id: [1; 16],
            from_user: [2; 16],
            to_user: [3; 16],
            at_ms: 99,
            text: "hello".into(),
            delivered: true,
        }
    }

    #[test]
    fn a_delivery_round_trips() {
        let frame = Frame::Deliver(sample());
        let decoded = decode(&encode(&frame).unwrap()).unwrap();
        match decoded {
            Frame::Deliver(message) => {
                assert_eq!(message.text, "hello");
                assert_eq!(message.id, [1; 16]);
            }
            Frame::DeliverAck { .. } => panic!("expected a delivery"),
        }
    }

    #[test]
    fn blank_text_is_not_a_message() {
        let mut message = sample();
        message.text = "   ".into();
        assert!(encode(&Frame::Deliver(message)).is_none());
    }
}
