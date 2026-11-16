//! App payloads. pNet carries them and does not read them.
//!
//! Each payload starts with a version byte and a type byte. Text is at most
//! [`MAX_TEXT`] bytes so one message stays inside a single datagram.

use crate::store::Message;

pub const VERSION: u8 = 1;
pub const MAX_TEXT: usize = 800;

pub const POST: u8 = 0x01;
pub const POST_ACK: u8 = 0x02;
pub const DELIVER: u8 = 0x03;
pub const DELIVER_ACK: u8 = 0x04;
pub const THREADS_REQ: u8 = 0x05;
pub const THREADS_RESP: u8 = 0x06;
pub const HISTORY_REQ: u8 = 0x07;
pub const HISTORY_RESP: u8 = 0x08;

pub const ACK_OK: u8 = 0;
pub const ACK_REJECTED: u8 = 1;

const HISTORY_LIMIT: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub id: [u8; 16],
    pub to_user: [u8; 16],
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadRow {
    pub peer: [u8; 16],
    pub at_ms: u64,
    pub preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Post(Post),
    PostAck { id: [u8; 16], status: u8 },
    Deliver(Message),
    DeliverAck { id: [u8; 16] },
    ThreadsReq,
    ThreadsResp(Vec<ThreadRow>),
    HistoryReq { peer: [u8; 16], after: [u8; 16] },
    HistoryResp {
        peer: [u8; 16],
        messages: Vec<Message>,
        more: bool,
    },
}

pub fn encode(frame: &Frame) -> Option<Vec<u8>> {
    let mut buf = vec![VERSION];
    match frame {
        Frame::Post(post) => {
            if post.text.len() > MAX_TEXT || !is_text(&post.text) {
                return None;
            }
            buf.push(POST);
            buf.extend_from_slice(&post.id);
            buf.extend_from_slice(&post.to_user);
            push_text(&mut buf, &post.text);
        }
        Frame::PostAck { id, status } => {
            buf.push(POST_ACK);
            buf.extend_from_slice(id);
            buf.push(*status);
        }
        Frame::Deliver(message) => {
            if message.text.len() > MAX_TEXT {
                return None;
            }
            buf.push(DELIVER);
            push_message(&mut buf, message);
        }
        Frame::DeliverAck { id } => {
            buf.push(DELIVER_ACK);
            buf.extend_from_slice(id);
        }
        Frame::ThreadsReq => buf.push(THREADS_REQ),
        Frame::ThreadsResp(rows) => {
            buf.push(THREADS_RESP);
            let n = rows.len().min(32);
            buf.push(n as u8);
            for row in rows.iter().take(n) {
                buf.extend_from_slice(&row.peer);
                buf.extend_from_slice(&row.at_ms.to_be_bytes());
                let preview = preview_of(&row.preview);
                buf.push(preview.len() as u8);
                buf.extend_from_slice(preview.as_bytes());
            }
        }
        Frame::HistoryReq { peer, after } => {
            buf.push(HISTORY_REQ);
            buf.extend_from_slice(peer);
            buf.extend_from_slice(after);
        }
        Frame::HistoryResp {
            peer,
            messages,
            more,
        } => {
            buf.push(HISTORY_RESP);
            buf.extend_from_slice(peer);
            let n = messages.len().min(HISTORY_LIMIT);
            buf.push(n as u8);
            for message in messages.iter().take(n) {
                if message.text.len() > MAX_TEXT {
                    return None;
                }
                push_message(&mut buf, message);
            }
            buf.push(u8::from(*more));
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
        POST => Some(Frame::Post(Post {
            id: cur.array()?,
            to_user: cur.array()?,
            text: cur.text()?,
        })),
        POST_ACK => Some(Frame::PostAck {
            id: cur.array()?,
            status: cur.u8()?,
        }),
        DELIVER => Some(Frame::Deliver(cur.message()?)),
        DELIVER_ACK => Some(Frame::DeliverAck { id: cur.array()? }),
        THREADS_REQ => Some(Frame::ThreadsReq),
        THREADS_RESP => {
            let n = cur.u8()? as usize;
            let mut rows = Vec::with_capacity(n);
            for _ in 0..n {
                rows.push(ThreadRow {
                    peer: cur.array()?,
                    at_ms: cur.u64()?,
                    preview: {
                        let len = cur.u8()? as usize;
                        let end = cur.pos.checked_add(len)?;
                        let slice = cur.data.get(cur.pos..end)?;
                        let text = std::str::from_utf8(slice).ok()?.to_string();
                        cur.pos = end;
                        text
                    },
                });
            }
            Some(Frame::ThreadsResp(rows))
        }
        HISTORY_REQ => Some(Frame::HistoryReq {
            peer: cur.array()?,
            after: cur.array()?,
        }),
        HISTORY_RESP => {
            let peer = cur.array()?;
            let n = cur.u8()? as usize;
            let mut messages = Vec::with_capacity(n);
            for _ in 0..n {
                messages.push(cur.message()?);
            }
            let more = cur.u8()? != 0;
            Some(Frame::HistoryResp {
                peer,
                messages,
                more,
            })
        }
        _ => None,
    }
}

/// The next history page. `after` is the last id the client already has.
pub fn history_page(messages: &[Message], after: &[u8; 16]) -> (Vec<Message>, bool) {
    let start = if *after == [0u8; 16] {
        0
    } else {
        match messages.iter().position(|message| message.id == *after) {
            Some(index) => index + 1,
            None => 0,
        }
    };
    let rest = messages.get(start..).unwrap_or(&[]);
    let n = rest.len().min(HISTORY_LIMIT);
    let more = rest.len() > HISTORY_LIMIT;
    (rest[..n].to_vec(), more)
}

pub fn is_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_TEXT && !text.contains('\0')
}

fn preview_of(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > 40 {
            break;
        }
        if ch == '\n' || ch == '\r' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
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
    fn u8(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }

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
    fn a_post_round_trips() {
        let frame = Frame::Post(Post {
            id: [9; 16],
            to_user: [4; 16],
            text: "ping".into(),
        });
        assert_eq!(decode(&encode(&frame).unwrap()).unwrap(), frame);
    }

    #[test]
    fn history_pages_three_at_a_time() {
        let messages: Vec<_> = (0..5)
            .map(|n| Message {
                id: [n; 16],
                ..sample()
            })
            .collect();
        let (page, more) = history_page(&messages, &[0; 16]);
        assert_eq!(page.len(), 3);
        assert!(more);
        let (rest, more) = history_page(&messages, &page[2].id);
        assert_eq!(rest.len(), 2);
        assert!(!more);
    }

    #[test]
    fn blank_text_is_not_a_message() {
        assert!(encode(&Frame::Post(Post {
            id: [1; 16],
            to_user: [2; 16],
            text: "   ".into(),
        }))
        .is_none());
    }
}
