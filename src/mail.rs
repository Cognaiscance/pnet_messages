//! What the server does with one client post or one delivery.
//!
//! The client posts to its own server. That server stores the message, then
//! delivers the same bytes to the other person's server. The other server
//! stores them for that person's clients.

use crate::proto::{self, Frame, Post, ACK_OK, ACK_REJECTED};
use crate::store::{Message, Store, StoreError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accept {
    pub ack: Vec<u8>,
    /// Present only the first time this id is stored.
    pub deliver: Option<Message>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Store a message this user is sending. Empty text is refused.
pub fn accept_post(
    store: &mut Store,
    owner: &[u8; 16],
    post: &Post,
    at_ms: u64,
) -> Result<Accept, StoreError> {
    if !proto::is_text(&post.text) || post.to_user == *owner || post.to_user == [0u8; 16] {
        return Ok(Accept {
            ack: ack(post.id, ACK_REJECTED),
            deliver: None,
        });
    }
    let before = store.messages().len();
    let saved = store
        .insert(Message {
            id: post.id,
            from_user: *owner,
            to_user: post.to_user,
            at_ms,
            text: post.text.clone(),
            delivered: false,
        })?
        .clone();
    let fresh = store.messages().len() > before;
    Ok(Accept {
        ack: ack(post.id, ACK_OK),
        deliver: fresh.then_some(saved),
    })
}

/// Store a message the other person's server delivered to this user.
pub fn accept_deliver(
    store: &mut Store,
    owner: &[u8; 16],
    message: &Message,
) -> Result<bool, StoreError> {
    if message.to_user != *owner || message.from_user == *owner || !proto::is_text(&message.text) {
        return Ok(false);
    }
    let before = store.messages().len();
    store.insert(Message {
        delivered: true,
        ..message.clone()
    })?;
    Ok(store.messages().len() > before)
}

fn ack(id: [u8; 16], status: u8) -> Vec<u8> {
    proto::encode(&Frame::PostAck { id, status }).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::decode;

    #[test]
    fn the_first_post_is_delivered_and_a_retry_is_not() {
        let dir = std::env::temp_dir().join(format!("pnet-messages-mail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).unwrap();
        let post = Post {
            id: [7; 16],
            to_user: [2; 16],
            text: "hello chad".into(),
        };
        let first = accept_post(&mut store, &[1; 16], &post, 10).unwrap();
        assert!(matches!(
            decode(&first.ack),
            Some(Frame::PostAck { status: ACK_OK, .. })
        ));
        assert_eq!(first.deliver.unwrap().text, "hello chad");
        let again = accept_post(&mut store, &[1; 16], &post, 11).unwrap();
        assert!(again.deliver.is_none());
        assert_eq!(store.messages().len(), 1);
        assert_eq!(store.messages()[0].at_ms, 10);

        let mut other = Store::open(&dir.join("other")).unwrap();
        let incoming = Message {
            id: [7; 16],
            from_user: [1; 16],
            to_user: [2; 16],
            at_ms: 10,
            text: "hello chad".into(),
            delivered: false,
        };
        assert!(accept_deliver(&mut other, &[2; 16], &incoming).unwrap());
        assert!(!accept_deliver(&mut other, &[2; 16], &incoming).unwrap());
        assert_eq!(other.between(&[2; 16], &[1; 16]).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_post_is_refused() {
        let dir = std::env::temp_dir().join(format!(
            "pnet-messages-blank-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).unwrap();
        let post = Post {
            id: [1; 16],
            to_user: [2; 16],
            text: " ".into(),
        };
        let accepted = accept_post(&mut store, &[9; 16], &post, 1).unwrap();
        assert!(accepted.deliver.is_none());
        assert!(store.messages().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
