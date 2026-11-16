//! The server's message file. Clients do not open it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"MSG1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: [u8; 16],
    pub from_user: [u8; 16],
    pub to_user: [u8; 16],
    pub at_ms: u64,
    pub text: String,
    /// The other person's server has acknowledged this copy.
    pub delivered: bool,
}

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Bad,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(err) => write!(f, "{err}"),
            StoreError::Bad => f.write_str("message file is not MSG1"),
        }
    }
}

pub struct Store {
    path: PathBuf,
    messages: Vec<Message>,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(dir).map_err(StoreError::Io)?;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        let path = dir.join("messages");
        let messages = if path.exists() {
            load(&path)?
        } else {
            Vec::new()
        };
        Ok(Self { path, messages })
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Insert `message` unless that id is already stored.
    ///
    /// A repeat returns the saved row and does not change it.
    pub fn insert(&mut self, message: Message) -> Result<&Message, StoreError> {
        if let Some(index) = self.messages.iter().position(|row| row.id == message.id) {
            return Ok(&self.messages[index]);
        }
        self.messages.push(message);
        self.save()?;
        Ok(self.messages.last().expect("just pushed"))
    }

    pub fn mark_delivered(&mut self, id: &[u8; 16]) -> Result<bool, StoreError> {
        let Some(row) = self.messages.iter_mut().find(|row| row.id == *id) else {
            return Ok(false);
        };
        if row.delivered {
            return Ok(false);
        }
        row.delivered = true;
        self.save()?;
        Ok(true)
    }

    pub fn between(&self, owner: &[u8; 16], peer: &[u8; 16]) -> Vec<Message> {
        self.messages
            .iter()
            .filter(|row| {
                (row.from_user == *owner && row.to_user == *peer)
                    || (row.from_user == *peer && row.to_user == *owner)
            })
            .cloned()
            .collect()
    }

    pub fn pending_delivery(&self, owner: &[u8; 16]) -> Vec<Message> {
        self.messages
            .iter()
            .filter(|row| row.from_user == *owner && !row.delivered)
            .cloned()
            .collect()
    }

    /// One row per other person, newest last-message first.
    pub fn threads(&self, owner: &[u8; 16]) -> Vec<Message> {
        let mut peers: Vec<[u8; 16]> = Vec::new();
        for row in &self.messages {
            let peer = if row.from_user == *owner {
                row.to_user
            } else if row.to_user == *owner {
                row.from_user
            } else {
                continue;
            };
            if !peers.contains(&peer) {
                peers.push(peer);
            }
        }
        let mut latest: Vec<Message> = peers
            .into_iter()
            .filter_map(|peer| self.between(owner, &peer).into_iter().last())
            .collect();
        latest.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.id.cmp(&b.id)));
        latest
    }

    fn save(&self) -> Result<(), StoreError> {
        let mut buf = Vec::from(MAGIC);
        let count = u32::try_from(self.messages.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&count.to_be_bytes());
        for message in self.messages.iter().take(count as usize) {
            buf.extend_from_slice(&message.id);
            buf.extend_from_slice(&message.from_user);
            buf.extend_from_slice(&message.to_user);
            buf.extend_from_slice(&message.at_ms.to_be_bytes());
            buf.push(u8::from(message.delivered));
            let len = u16::try_from(message.text.len()).unwrap_or(u16::MAX);
            buf.extend_from_slice(&len.to_be_bytes());
            buf.extend_from_slice(&message.text.as_bytes()[..len as usize]);
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.path)
            .map_err(StoreError::Io)?;
        file.write_all(&buf).map_err(StoreError::Io)?;
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).map_err(StoreError::Io)?;
        Ok(())
    }
}

fn load(path: &Path) -> Result<Vec<Message>, StoreError> {
    let bytes = fs::read(path).map_err(StoreError::Io)?;
    if bytes.len() < 8 || &bytes[..4] != MAGIC {
        return Err(StoreError::Bad);
    }
    let count = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let mut pos = 8;
    let mut messages = Vec::with_capacity(count);
    for _ in 0..count {
        let id = take::<16>(&bytes, &mut pos)?;
        let from_user = take::<16>(&bytes, &mut pos)?;
        let to_user = take::<16>(&bytes, &mut pos)?;
        let at = take::<8>(&bytes, &mut pos)?;
        let delivered = *bytes.get(pos).ok_or(StoreError::Bad)? != 0;
        pos += 1;
        let len_bytes = take::<2>(&bytes, &mut pos)?;
        let len = u16::from_be_bytes(len_bytes) as usize;
        let end = pos.checked_add(len).ok_or(StoreError::Bad)?;
        let slice = bytes.get(pos..end).ok_or(StoreError::Bad)?;
        let text = std::str::from_utf8(slice)
            .map_err(|_| StoreError::Bad)?
            .to_string();
        pos = end;
        messages.push(Message {
            id,
            from_user,
            to_user,
            at_ms: u64::from_be_bytes(at),
            text,
            delivered,
        });
    }
    if pos != bytes.len() {
        return Err(StoreError::Bad);
    }
    Ok(messages)
}

fn take<const N: usize>(bytes: &[u8], pos: &mut usize) -> Result<[u8; N], StoreError> {
    let end = pos.checked_add(N).ok_or(StoreError::Bad)?;
    let slice = bytes.get(*pos..end).ok_or(StoreError::Bad)?;
    *pos = end;
    slice.try_into().map_err(|_| StoreError::Bad)
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

pub fn parse_hex_id(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (index, chunk) in text.as_bytes().chunks(2).enumerate() {
        let hi = from_hex(chunk[0])?;
        let lo = from_hex(chunk[1])?;
        out[index] = (hi << 4) | lo;
    }
    Some(out)
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: u8, text: &str) -> Message {
        Message {
            id: [id; 16],
            from_user: [1; 16],
            to_user: [2; 16],
            at_ms: u64::from(id),
            text: text.into(),
            delivered: false,
        }
    }

    #[test]
    fn a_repeat_id_keeps_the_first_text() {
        let dir = std::env::temp_dir().join(format!("pnet-messages-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).unwrap();
        store.insert(message(1, "first")).unwrap();
        let saved = store.insert(message(1, "second")).unwrap();
        assert_eq!(saved.text, "first");
        store.mark_delivered(&[1; 16]).unwrap();
        let reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.messages().len(), 1);
        assert!(reopened.messages()[0].delivered);
        assert_eq!(reopened.messages()[0].text, "first");
        let _ = fs::remove_dir_all(&dir);
    }
}
