//! The local node's get-data tree.
//!
//! Layout matches pNet's app get-data encoder. Certificate fields sit between
//! a device's host list and its app list.

use crate::{CLIENT_ALIAS, SERVER_ALIAS};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Device,
    Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRecord {
    pub id: [u8; 16],
    pub alias: String,
    pub approved: bool,
    pub push_port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRecord {
    pub id: [u8; 16],
    pub alias: String,
    pub grade: Grade,
    pub sg_rank: u8,
    pub apps: Vec<AppRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRecord {
    pub alias: String,
    pub id: [u8; 16],
    pub devices: Vec<DeviceRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    pub local_app_id: [u8; 16],
    pub local_app_alias: String,
    pub local_app_approved: bool,
    pub token: [u8; 16],
    pub local_device: [u8; 16],
    pub owner_alias: String,
    pub owner_id: [u8; 16],
    pub own_devices: Vec<DeviceRecord>,
    pub contacts: Vec<ContactRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub device: [u8; 16],
    pub app: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadUtf8,
    BadGrade,
    Trailing,
    Status,
    Rejected { code: Option<u8> },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Truncated => f.write_str("truncated get-data reply"),
            ParseError::BadUtf8 => f.write_str("get-data string is not utf-8"),
            ParseError::BadGrade => f.write_str("device grade is not device or server"),
            ParseError::Trailing => f.write_str("get-data reply has trailing bytes"),
            ParseError::Status => f.write_str("get-data status is not success"),
            ParseError::Rejected { code: Some(code) } => {
                write!(f, "node rejected get-data ({code:#04x})")
            }
            ParseError::Rejected { code: None } => f.write_str("node rejected get-data"),
        }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, ParseError> {
        let byte = *self.data.get(self.pos).ok_or(ParseError::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        let end = self.pos.checked_add(N).ok_or(ParseError::Truncated)?;
        let slice = self.data.get(self.pos..end).ok_or(ParseError::Truncated)?;
        let array = slice.try_into().map_err(|_| ParseError::Truncated)?;
        self.pos = end;
        Ok(array)
    }

    fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        let end = self.pos.checked_add(n).ok_or(ParseError::Truncated)?;
        if end > self.data.len() {
            return Err(ParseError::Truncated);
        }
        self.pos = end;
        Ok(())
    }

    fn str(&mut self) -> Result<String, ParseError> {
        let len = self.u8()? as usize;
        let end = self.pos.checked_add(len).ok_or(ParseError::Truncated)?;
        let bytes = self.data.get(self.pos..end).ok_or(ParseError::Truncated)?;
        let text = std::str::from_utf8(bytes).map_err(|_| ParseError::BadUtf8)?;
        self.pos = end;
        Ok(text.to_string())
    }
}

pub fn parse_get_data(reply: &[u8]) -> Result<Directory, ParseError> {
    let mut cur = Cursor { data: reply, pos: 0 };
    let status = cur.u8()?;
    if status == 0x01 {
        return Err(ParseError::Rejected {
            code: reply.get(cur.pos).copied(),
        });
    }
    if status != 0x00 {
        return Err(ParseError::Status);
    }

    let local_app_id = cur.array()?;
    let local_app_alias = cur.str()?;
    cur.skip(4 + 2)?;
    let local_app_approved = cur.u8()? != 0;
    let token = cur.array()?;
    let local_device = cur.array()?;
    let owner_alias = cur.str()?;
    let owner_id = cur.array()?;

    let device_count = cur.u8()? as usize;
    let mut own_devices = Vec::with_capacity(device_count);
    for _ in 0..device_count {
        let mut device = read_device(&mut cur)?;
        device.apps = read_apps(&mut cur, true)?;
        own_devices.push(device);
    }

    let contact_count = cur.u8()? as usize;
    let mut contacts = Vec::with_capacity(contact_count);
    for _ in 0..contact_count {
        let alias = cur.str()?;
        let id = cur.array()?;
        let device_count = cur.u8()? as usize;
        let mut devices = Vec::with_capacity(device_count);
        for _ in 0..device_count {
            let mut device = read_device(&mut cur)?;
            device.apps = read_apps(&mut cur, false)?;
            devices.push(device);
        }
        contacts.push(ContactRecord { alias, id, devices });
    }

    if cur.pos != reply.len() {
        return Err(ParseError::Trailing);
    }

    Ok(Directory {
        local_app_id,
        local_app_alias,
        local_app_approved,
        token,
        local_device,
        owner_alias,
        owner_id,
        own_devices,
        contacts,
    })
}

fn read_device(cur: &mut Cursor<'_>) -> Result<DeviceRecord, ParseError> {
    let id = cur.array()?;
    let alias = cur.str()?;
    let grade = match cur.u8()? {
        0 => Grade::Device,
        1 => Grade::Server,
        _ => return Err(ParseError::BadGrade),
    };
    let sg_rank = cur.u8()?;
    let host_count = cur.u8()?;
    for _ in 0..host_count {
        let _host = cur.str()?;
    }
    cur.skip(32 + 32 + 64 + 8)?;
    let _cert_alias = cur.str()?;
    Ok(DeviceRecord {
        id,
        alias,
        grade,
        sg_rank,
        apps: Vec::new(),
    })
}

fn read_apps(cur: &mut Cursor<'_>, with_host: bool) -> Result<Vec<AppRecord>, ParseError> {
    let count = cur.u8()? as usize;
    let mut apps = Vec::with_capacity(count);
    for _ in 0..count {
        let id = cur.array()?;
        let alias = cur.str()?;
        let (approved, push_port) = if with_host {
            cur.skip(4)?;
            let port = u16::from_be_bytes(cur.array()?);
            (cur.u8()? != 0, Some(port))
        } else {
            (true, None)
        };
        apps.push(AppRecord {
            id,
            alias,
            approved,
            push_port,
        });
    }
    Ok(apps)
}

fn runs_server(dir: &Directory, device: &DeviceRecord) -> bool {
    if device.id == dir.local_device && dir.local_app_alias == SERVER_ALIAS {
        return true;
    }
    device.apps.iter().any(|app| app.alias == SERVER_ALIAS)
}

fn rank_key(device: &DeviceRecord) -> (bool, u8, [u8; 16]) {
    (device.sg_rank == 0, device.sg_rank, device.id)
}

fn server_peer(device: &DeviceRecord) -> Option<Peer> {
    let app = device
        .apps
        .iter()
        .find(|app| app.alias == SERVER_ALIAS && app.approved)?;
    Some(Peer {
        device: device.id,
        app: app.id,
    })
}

/// Top-ranked own server-grade device that runs `messages-server`.
pub fn record_holder(dir: &Directory) -> Option<&DeviceRecord> {
    dir.own_devices
        .iter()
        .filter(|device| device.grade == Grade::Server && runs_server(dir, device))
        .min_by(|a, b| rank_key(a).cmp(&rank_key(b)))
}

pub fn holds_record(dir: &Directory) -> bool {
    record_holder(dir).is_some_and(|holder| holder.id == dir.local_device)
}

/// Approved `messages-server` on this user's top-ranked server-grade node.
pub fn own_server(dir: &Directory) -> Option<Peer> {
    server_peer(record_holder(dir)?)
}

/// Approved `messages-server` on a contact's top-ranked server-grade node.
pub fn contact_server(dir: &Directory, user: &[u8; 16]) -> Option<Peer> {
    let contact = dir.contacts.iter().find(|contact| contact.id == *user)?;
    let device = contact
        .devices
        .iter()
        .filter(|device| device.grade == Grade::Server && runs_server_contact(device))
        .min_by(|a, b| rank_key(a).cmp(&rank_key(b)))?;
    server_peer(device)
}

fn runs_server_contact(device: &DeviceRecord) -> bool {
    device.apps.iter().any(|app| app.alias == SERVER_ALIAS)
}

/// The contact whose `messages-server` app id is `app`.
pub fn contact_of_server_app<'a>(dir: &'a Directory, app: &[u8; 16]) -> Option<&'a ContactRecord> {
    dir.contacts.iter().find(|contact| {
        contact.devices.iter().any(|device| {
            device.apps.iter().any(|candidate| {
                candidate.id == *app && candidate.alias == SERVER_ALIAS
            })
        })
    })
}

pub fn contact_by_id<'a>(dir: &'a Directory, user: &[u8; 16]) -> Option<&'a ContactRecord> {
    dir.contacts.iter().find(|contact| contact.id == *user)
}

pub fn own_device_of_app(dir: &Directory, app: &[u8; 16]) -> Option<[u8; 16]> {
    dir.own_devices.iter().find_map(|device| {
        device
            .apps
            .iter()
            .any(|candidate| candidate.id == *app)
            .then_some(device.id)
    })
}

/// Approved `messages-client` on one of this user's own devices.
pub fn approved_own_client(dir: &Directory, app: &[u8; 16]) -> bool {
    dir.own_devices.iter().any(|device| {
        device.apps.iter().any(|candidate| {
            candidate.id == *app && candidate.approved && candidate.alias == CLIENT_ALIAS
        })
    })
}

pub fn alias_for(dir: &Directory, user: &[u8; 16]) -> String {
    if *user == dir.owner_id {
        return dir.owner_alias.clone();
    }
    contact_by_id(dir, user)
        .map(|contact| contact.alias.clone())
        .filter(|alias| !alias.is_empty())
        .unwrap_or_else(|| crate::store::hex(&user[..4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_str(buf: &mut Vec<u8>, text: &str) {
        buf.push(text.len() as u8);
        buf.extend_from_slice(text.as_bytes());
    }

    fn push_device(buf: &mut Vec<u8>, id: [u8; 16], alias: &str, grade: u8, rank: u8) {
        buf.extend_from_slice(&id);
        push_str(buf, alias);
        buf.push(grade);
        buf.push(rank);
        buf.push(0);
        buf.extend_from_slice(&[0u8; 32 + 32 + 64 + 8]);
        push_str(buf, alias);
    }

    fn push_own_app(buf: &mut Vec<u8>, id: [u8; 16], alias: &str, port: u16, approved: bool) {
        buf.extend_from_slice(&id);
        push_str(buf, alias);
        buf.extend_from_slice(&[127, 0, 0, 1]);
        buf.extend_from_slice(&port.to_be_bytes());
        buf.push(u8::from(approved));
    }

    fn push_contact_app(buf: &mut Vec<u8>, id: [u8; 16], alias: &str) {
        buf.extend_from_slice(&id);
        push_str(buf, alias);
    }

    fn sample() -> Vec<u8> {
        let mut buf = vec![0x00];
        buf.extend_from_slice(&[0x11; 16]);
        push_str(&mut buf, SERVER_ALIAS);
        buf.extend_from_slice(&[127, 0, 0, 1]);
        buf.extend_from_slice(&8831u16.to_be_bytes());
        buf.push(1);
        buf.extend_from_slice(&[0x22; 16]);
        buf.extend_from_slice(&[0xA1; 16]);
        push_str(&mut buf, "william");
        buf.extend_from_slice(&[0xB1; 16]);

        buf.push(2);
        push_device(&mut buf, [0xA1; 16], "n64", 1, 1);
        buf.push(2);
        push_own_app(&mut buf, [0x11; 16], SERVER_ALIAS, 8831, true);
        push_own_app(&mut buf, [0x31; 16], CLIENT_ALIAS, 8830, true);
        push_device(&mut buf, [0xA2; 16], "laptop", 0, 0);
        buf.push(1);
        push_own_app(&mut buf, [0x32; 16], CLIENT_ALIAS, 8830, true);

        buf.push(1);
        push_str(&mut buf, "chad");
        buf.extend_from_slice(&[0xC1; 16]);
        buf.push(2);
        push_device(&mut buf, [0xD1; 16], "x1", 1, 1);
        buf.push(1);
        push_contact_app(&mut buf, [0x41; 16], SERVER_ALIAS);
        push_device(&mut buf, [0xD2; 16], "phone", 0, 0);
        buf.push(1);
        push_contact_app(&mut buf, [0x42; 16], CLIENT_ALIAS);
        buf
    }

    #[test]
    fn own_server_is_rank_one_and_the_contact_server_is_separate() {
        let dir = parse_get_data(&sample()).unwrap();
        assert_eq!(own_server(&dir).unwrap().app, [0x11; 16]);
        assert!(holds_record(&{
            let mut dir = dir.clone();
            dir.local_device = [0xA1; 16];
            dir
        }));
        let peer = contact_server(&dir, &[0xC1; 16]).unwrap();
        assert_eq!(peer.device, [0xD1; 16]);
        assert_eq!(peer.app, [0x41; 16]);
        assert!(contact_server(&dir, &[0xFF; 16]).is_none());
        assert_eq!(
            contact_of_server_app(&dir, &[0x41; 16]).unwrap().alias,
            "chad"
        );
        assert!(approved_own_client(&dir, &[0x32; 16]));
        assert!(!approved_own_client(&dir, &[0x42; 16]));
    }
}
