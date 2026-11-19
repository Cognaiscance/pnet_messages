//! What the server does with one send from the page or one delivery.
//!
//! The page writes a message for a contact. This server stores it, then
//! delivers the same bytes to that contact's server. The other server stores
//! them for that person's page.

use crate::proto;
use crate::store::{Message, Store, StoreError};

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Store a message this user is sending. Empty text is refused.
/// `Some` only the first time this id is stored.
pub fn accept_send(
    store: &mut Store,
    owner: &[u8; 16],
    id: [u8; 16],
    to_user: [u8; 16],
    text: &str,
    at_ms: u64,
) -> Result<Option<Message>, StoreError> {
    if !proto::is_text(text) || to_user == *owner || to_user == [0u8; 16] {
        return Ok(None);
    }
    let before = store.messages().len();
    let saved = store
        .insert(Message {
            id,
            from_user: *owner,
            to_user,
            at_ms,
            text: text.to_string(),
            delivered: false,
        })?
        .clone();
    let fresh = store.messages().len() > before;
    Ok(fresh.then_some(saved))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_send_is_delivered_and_a_retry_is_not() {
        let dir = std::env::temp_dir().join(format!("pnet-messages-mail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).unwrap();
        let first = accept_send(&mut store, &[1; 16], [7; 16], [2; 16], "hello chad", 10).unwrap();
        assert_eq!(first.unwrap().text, "hello chad");
        let again = accept_send(&mut store, &[1; 16], [7; 16], [2; 16], "hello chad", 11).unwrap();
        assert!(again.is_none());
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
        let accepted = accept_send(&mut store, &[9; 16], [1; 16], [2; 16], " ", 1).unwrap();
        assert!(accepted.is_none());
        assert!(store.messages().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
