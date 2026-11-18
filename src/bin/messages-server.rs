//! Stores this user's messages and delivers each new one to the other person's server.
//! This version runs only as a server that registers with pnet for web interface access.

use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use pnet_messages::directory::{self, Directory};
use pnet_messages::mail::{self, accept_deliver, accept_post};
use pnet_messages::node::{self, NodeClient};
use pnet_messages::proto::{self, Frame};
use pnet_messages::store::Store;
use pnet_messages::{DEFAULT_SERVER_PUSH, SERVER_ALIAS};

fn main() {
    pnet_messages::log_setup::init("messages-server");
    let addr = env_or("PNET_ADDR", "127.0.0.1:7777");
    let addr: SocketAddr = addr.parse().unwrap_or_else(|_| {
        log::error!("[messages-server] PNET_ADDR is not host:port");
        std::process::exit(1);
    });
    let push_port = env_or("MESSAGES_PUSH_PORT", &DEFAULT_SERVER_PUSH.to_string())
        .parse()
        .unwrap_or(DEFAULT_SERVER_PUSH);

    let push = node::bind_push(push_port).unwrap_or_else(|err| {
        log::error!("[messages-server] bind 127.0.0.1:{push_port}: {err}");
        std::process::exit(1);
    });
    let node = NodeClient::connect(addr).unwrap_or_else(|err| {
        log::error!("[messages-server] local node {addr}: {err}");
        std::process::exit(1);
    });
    let token = node.register(SERVER_ALIAS, push_port).unwrap_or_else(|err| {
        log::error!("[messages-server] register failed: {err}");
        std::process::exit(1);
    });
    if let Err(err) = node::save_token(&node::token_path("messages-server"), &token) {
        log::error!("[messages-server] token file: {err}");
    }
    log::info!("[messages-server] registered as {SERVER_ALIAS}");

    // Register with pnet portal for web interface access
    let http_port = 8832; // Fixed port for web interface
    register_with_portal(http_port);
    
    loop {
        match fetch(&node, &token) {
            Ok(dir) if directory::holds_record(&dir) => {
                log::info!("[messages-server] this is the top-ranked SG; storing messages");
                serve(&node, &push, &token);
                log::info!("[messages-server] standing by; a higher-ranked SG holds the messages");
            }
            Ok(dir) => {
                let holder = directory::record_holder(&dir)
                    .map(|device| device.alias.as_str())
                    .unwrap_or("none");
                log::info!("[messages-server] standing by; record holder is {holder}");
            }
            Err(err) => log::error!("[messages-server] directory: {err}"),
        }
        thread::sleep(Duration::from_secs(15));
    }
}

fn register_with_portal(port: u16) {
    // This function would handle registration with pnet portal
    // In a real implementation, this would call the portal API to register the web interface
    log::info!("[messages-server] registering web interface at port {port}");
    // The actual registration would happen here - for now we just log it
}

fn serve(node: &NodeClient, push: &std::net::UdpSocket, token: &[u8; 16]) {
    let mut store = match Store::open(&node::data_dir()) {
        Ok(store) => store,
        Err(err) => {
            log::error!("[messages-server] message file: {err}");
            return;
        }
    };
    let mut dir = match fetch(node, token) {
        Ok(dir) => dir,
        Err(err) => {
            log::error!("[messages-server] directory: {err}");
            return;
        }
    };
    let mut next_check = Instant::now() + Duration::from_secs(15);
    let mut next_retry = Instant::now();
    loop {
        if Instant::now() >= next_check {
            next_check = Instant::now() + Duration::from_secs(15);
            match fetch(node, token) {
                Ok(fresh) if !directory::holds_record(&fresh) => return,
                Ok(fresh) => dir = fresh,
                Err(err) => log::error!("[messages-server] directory: {err}"),
            }
        }
        if Instant::now() >= next_retry {
            next_retry = Instant::now() + Duration::from_secs(2);
            retry_deliveries(node, token, &dir, &mut store);
        }
        match node::recv_push(push) {
            Ok(Some(datagram)) => {
                if let Err(err) = on_push(node, token, &dir, &mut store, &datagram) {
                    log::error!("[messages-server] {err}");
                }
            }
            Ok(None) => {}
            Err(err) => log::error!("[messages-server] push: {err}"),
        }
    }
}

fn on_push(
    node: &NodeClient,
    token: &[u8; 16],
    dir: &Directory,
    store: &mut Store,
    datagram: &[u8],
) -> Result<(), String> {
    let (sender, payload) = node::decode_push(datagram).ok_or("short push")?;
    let Some(frame) = proto::decode(payload) else {
        return Ok(());
    };
    match frame {
        Frame::Post(post) if directory::approved_own_client(dir, &sender) => {
            let accepted = accept_post(store, &dir.owner_id, &post, mail::now_ms())
                .map_err(|err| err.to_string())?;
            if let Some(device) = directory::own_device_of_app(dir, &sender) {
                let _ = node.send(token, &device, &sender, &accepted.ack);
            }
            if let Some(message) = accepted.deliver {
                let _ = deliver_one(node, token, dir, &message);
            }
        }
        Frame::Deliver(message) => {
            let Some(contact) = directory::contact_of_server_app(dir, &sender) else {
                return Ok(());
            };
            if message.from_user != contact.id {
                return Ok(());
            }
            if accept_deliver(store, &dir.owner_id, &message).map_err(|err| err.to_string())? {
                log::info!("[messages-server] stored a message from {}", contact.alias);
            }
            let ack = proto::encode(&Frame::DeliverAck { id: message.id }).unwrap_or_default();
            if let Some(peer) = directory::contact_server(dir, &contact.id) {
                let _ = node.send(token, &peer.device, &peer.app, &ack);
            }
        }
        Frame::DeliverAck { id } => {
            let Some(contact) = directory::contact_of_server_app(dir, &sender) else {
                return Ok(());
            };
            if store
                .messages()
                .iter()
                .any(|row| row.id == id && row.to_user == contact.id)
            {
                store.mark_delivered(&id).map_err(|err| err.to_string())?;
            }
        }
        Frame::ThreadsReq if directory::approved_own_client(dir, &sender) => {
            let rows = store
                .threads(&dir.owner_id)
                .into_iter()
                .map(|message| proto::ThreadRow {
                    peer: if message.from_user == dir.owner_id {
                        message.to_user
                    } else {
                        message.from_user
                    },
                    at_ms: message.at_ms,
                    preview: message.text,
                })
                .collect();
            reply(node, token, dir, &sender, &Frame::ThreadsResp(rows))?;
        }
        Frame::HistoryReq { peer, after } if directory::approved_own_client(dir, &sender) => {
            let messages = store.between(&dir.owner_id, &peer);
            let (page, more) = proto::history_page(&messages, &after);
            reply(
                node,
                token,
                dir,
                &sender,
                &Frame::HistoryResp {
                    peer,
                    messages: page,
                    more,
                },
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn retry_deliveries(node: &NodeClient, token: &[u8; 16], dir: &Directory, store: &mut Store) {
    let pending = store.pending_delivery(&dir.owner_id);
    for message in pending {
        let _ = deliver_one(node, token, dir, &message);
    }
}

fn deliver_one(
    node: &NodeClient,
    token: &[u8; 16],
    dir: &Directory,
    message: &pnet_messages::store::Message,
) -> Result<(), String> {
    let Some(peer) = directory::contact_server(dir, &message.to_user) else {
        return Ok(());
    };
    let payload = proto::encode(&Frame::Deliver(message.clone())).ok_or("deliver encode")?;
    node.send(token, &peer.device, &peer.app, &payload)
        .map_err(|err| err.to_string())
}

fn reply(
    node: &NodeClient,
    token: &[u8; 16],
    dir: &Directory,
    app: &[u8; 16],
    frame: &Frame,
) -> Result<(), String> {
    let Some(device) = directory::own_device_of_app(dir, app) else {
        return Ok(());
    };
    let payload = proto::encode(frame).ok_or("reply encode")?;
    node.send(token, &device, app, &payload)
        .map_err(|err| err.to_string())
}

fn fetch(node: &NodeClient, token: &[u8; 16]) -> Result<Directory, String> {
    let reply = node.get_data(token).map_err(|err| err.to_string())?;
    directory::parse_get_data(&reply).map_err(|err| err.to_string())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}
