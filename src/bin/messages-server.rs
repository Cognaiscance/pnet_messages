//! Stores this user's messages, shows them on the owner portal, and delivers
//! each new one to the other person's server. The page lists contacts only.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pnet_messages::directory::{self, Directory};
use pnet_messages::mail::{self, accept_deliver, accept_send};
use pnet_messages::node::{self, NodeClient};
use pnet_messages::page::{self, Person};
use pnet_messages::proto::{self, Frame};
use pnet_messages::store::{self, Message, Store};
use pnet_messages::{DEFAULT_HTTP_PORT, DEFAULT_SERVER_PUSH, SERVER_ALIAS, WEB_SLUG, WEB_TITLE};

fn main() {
    pnet_messages::log_setup::init("messages-server");
    let addr = env_or("PNET_ADDR", "127.0.0.1:7777");
    let addr = addr.parse().unwrap_or_else(|_| {
        log::error!("[messages-server] PNET_ADDR is not host:port");
        std::process::exit(1);
    });
    let push_port = env_u16("MESSAGES_PUSH_PORT", DEFAULT_SERVER_PUSH);
    let http_port = env_u16("MESSAGES_HTTP_PORT", DEFAULT_HTTP_PORT);
    let portal = env_or("PNET_PORTAL", "http://127.0.0.1:8777");

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

    let node = Arc::new(Mutex::new(node));
    loop {
        match fetch(&node, &token) {
            Ok(dir) if directory::holds_record(&dir) => {
                log::info!("[messages-server] this is the top-ranked SG; storing messages");
                serve(&node, &push, &token, &portal, http_port, dir);
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

fn serve(
    node: &Arc<Mutex<NodeClient>>,
    push: &std::net::UdpSocket,
    token: &[u8; 16],
    portal: &str,
    http_port: u16,
    dir: Directory,
) {
    let store = match Store::open(&node::data_dir()) {
        Ok(store) => Arc::new(Mutex::new(store)),
        Err(err) => {
            log::error!("[messages-server] message file: {err}");
            return;
        }
    };
    let dir = Arc::new(Mutex::new(dir));
    let stop = Arc::new(AtomicBool::new(false));
    let listener = match TcpListener::bind(("127.0.0.1", http_port)) {
        Ok(listener) => Some(listener),
        Err(err) => {
            log::error!("[messages-server] http 127.0.0.1:{http_port}: {err}");
            None
        }
    };
    if let Some(listener) = listener {
        listener.set_nonblocking(true).ok();
        let page_store = Arc::clone(&store);
        let page_dir = Arc::clone(&dir);
        let page_node = Arc::clone(node);
        let page_stop = Arc::clone(&stop);
        let token = *token;
        thread::spawn(move || {
            http_loop(listener, page_store, page_dir, page_node, token, page_stop)
        });
    }

    let mut mounted = false;
    for attempt in 1..=5 {
        match portal_register(portal, http_port) {
            Ok(()) => {
                log::info!("[messages-server] portal /apps/{WEB_SLUG}/ → 127.0.0.1:{http_port}");
                mounted = true;
                break;
            }
            Err(err) => {
                log::error!("[messages-server] portal register attempt {attempt}: {err}");
                thread::sleep(Duration::from_millis(400));
            }
        }
    }

    let mut next_check = Instant::now() + Duration::from_secs(15);
    let mut next_retry = Instant::now();
    loop {
        if Instant::now() >= next_check {
            next_check = Instant::now() + Duration::from_secs(15);
            match fetch(node, token) {
                Ok(fresh) if !directory::holds_record(&fresh) => break,
                Ok(fresh) => *dir.lock().unwrap_or_else(|err| err.into_inner()) = fresh,
                Err(err) => log::error!("[messages-server] directory: {err}"),
            }
        }
        if Instant::now() >= next_retry {
            next_retry = Instant::now() + Duration::from_secs(2);
            let snapshot = dir.lock().unwrap_or_else(|err| err.into_inner()).clone();
            let mut store = store.lock().unwrap_or_else(|err| err.into_inner());
            retry_deliveries(node, token, &snapshot, &mut store);
        }
        match node::recv_push(push) {
            Ok(Some(datagram)) => {
                let snapshot = dir.lock().unwrap_or_else(|err| err.into_inner()).clone();
                let mut store = store.lock().unwrap_or_else(|err| err.into_inner());
                if let Err(err) = on_push(node, token, &snapshot, &mut store, &datagram) {
                    log::error!("[messages-server] {err}");
                }
            }
            Ok(None) => {}
            Err(err) => log::error!("[messages-server] push: {err}"),
        }
    }

    stop.store(true, Ordering::SeqCst);
    if mounted {
        if let Err(err) = portal_unregister(portal) {
            log::error!("[messages-server] portal unregister: {err}");
        }
    }
}

fn on_push(
    node: &Mutex<NodeClient>,
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
                let _ = send(node, token, &peer.device, &peer.app, &ack);
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
    }
    Ok(())
}

fn retry_deliveries(
    node: &Mutex<NodeClient>,
    token: &[u8; 16],
    dir: &Directory,
    store: &mut Store,
) {
    let pending = store.pending_delivery(&dir.owner_id);
    for message in pending {
        if directory::contact_by_id(dir, &message.to_user).is_none() {
            continue;
        }
        let _ = deliver_one(node, token, dir, &message);
    }
}

fn deliver_one(
    node: &Mutex<NodeClient>,
    token: &[u8; 16],
    dir: &Directory,
    message: &Message,
) -> Result<(), String> {
    let Some(peer) = directory::contact_server(dir, &message.to_user) else {
        return Ok(());
    };
    let payload = proto::encode(&Frame::Deliver(message.clone())).ok_or("deliver encode")?;
    send(node, token, &peer.device, &peer.app, &payload)
}

fn send(
    node: &Mutex<NodeClient>,
    token: &[u8; 16],
    device: &[u8; 16],
    app: &[u8; 16],
    payload: &[u8],
) -> Result<(), String> {
    node.lock()
        .unwrap_or_else(|err| err.into_inner())
        .send(token, device, app, payload)
        .map_err(|err| err.to_string())
}

fn fetch(node: &Mutex<NodeClient>, token: &[u8; 16]) -> Result<Directory, String> {
    let reply = node
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get_data(token)
        .map_err(|err| err.to_string())?;
    directory::parse_get_data(&reply).map_err(|err| err.to_string())
}

fn http_loop(
    listener: TcpListener,
    store: Arc<Mutex<Store>>,
    dir: Arc<Mutex<Directory>>,
    node: Arc<Mutex<NodeClient>>,
    token: [u8; 16],
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let store = Arc::clone(&store);
                let dir = Arc::clone(&dir);
                let node = Arc::clone(&node);
                thread::spawn(move || {
                    if let Err(err) = handle(stream, &store, &dir, &node, &token) {
                        log::error!("[messages-server] http: {err}");
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(40));
            }
            Err(err) => {
                log::error!("[messages-server] accept: {err}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle(
    mut stream: TcpStream,
    store: &Mutex<Store>,
    dir: &Mutex<Directory>,
    node: &Mutex<NodeClient>,
    token: &[u8; 16],
) -> Result<(), String> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let req = read_request(&mut stream)?;
    let first = req.lines().next().unwrap_or("");
    if first.contains("favicon") {
        stream
            .write_all(page::http_response("404 Not Found", "text/plain", "").as_slice())
            .ok();
        return Ok(());
    }
    let query = first.split_whitespace().nth(1).unwrap_or("/");
    let query = query.split_once('?').map(|(_, q)| q).unwrap_or("");
    let with = query_value(query, "with").and_then(|text| store::parse_hex_id(&text));

    let snapshot = dir.lock().unwrap_or_else(|err| err.into_inner()).clone();
    if first.starts_with("POST ") {
        let body = req.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or("");
        let peer = store::parse_hex_id(&form_value(body, "with")).or(with);
        let text = form_value(body, "text");
        if let Some(peer_id) = peer {
            if directory::contact_by_id(&snapshot, &peer_id).is_some() && proto::is_text(&text) {
                let mut messages = store.lock().unwrap_or_else(|err| err.into_inner());
                if let Some(message) = accept_send(
                    &mut messages,
                    &snapshot.owner_id,
                    fresh_id(),
                    peer_id,
                    &text,
                    mail::now_ms(),
                )
                .map_err(|err| err.to_string())?
                {
                    let _ = deliver_one(node, token, &snapshot, &message);
                }
            }
        }
        let location = match peer {
            Some(peer_id) => format!("?with={}&raw=1", store::hex(&peer_id)),
            None => "?raw=1".into(),
        };
        let resp = format!(
            "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(resp.as_bytes()).ok();
        return Ok(());
    }

    let messages = store.lock().unwrap_or_else(|err| err.into_inner());
    let people = people_of(&snapshot, &messages);
    let open = with
        .and_then(|id| people.iter().find(|person| person.id == id))
        .cloned();
    let lines = open
        .as_ref()
        .map(|person| {
            let history = messages.between(&snapshot.owner_id, &person.id);
            page::lines_for(
                &snapshot.owner_alias,
                &person.alias,
                &snapshot.owner_id,
                &history,
            )
        })
        .unwrap_or_default();
    let html = page::render(
        &snapshot.owner_alias,
        &people,
        open.as_ref(),
        &lines,
        "",
    );
    stream
        .write_all(&page::http_response(
            "200 OK",
            "text/html; charset=utf-8",
            &html,
        ))
        .map_err(|err| err.to_string())
}

fn people_of(dir: &Directory, store: &Store) -> Vec<Person> {
    let threads = store.threads(&dir.owner_id);
    let mut people: Vec<Person> = dir
        .contacts
        .iter()
        .map(|contact| {
            let latest = threads.iter().find(|message| {
                let peer = if message.from_user == dir.owner_id {
                    message.to_user
                } else {
                    message.from_user
                };
                peer == contact.id
            });
            Person {
                id: contact.id,
                alias: if contact.alias.is_empty() {
                    store::hex(&contact.id[..4])
                } else {
                    contact.alias.clone()
                },
                preview: latest.map(|message| message.text.clone()).unwrap_or_default(),
                at_ms: latest.map(|message| message.at_ms).unwrap_or(0),
            }
        })
        .collect();
    people.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.alias.cmp(&b.alias)));
    people
}

fn fresh_id() -> [u8; 16] {
    let ms = mail::now_ms();
    let extra = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(1);
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&ms.to_be_bytes());
    id[8..12].copy_from_slice(&extra.to_be_bytes());
    id[12..].copy_from_slice(&(std::process::id() as u32).to_be_bytes());
    id
}

fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut buf = vec![0u8; 8192];
    let mut n = 0;
    while n < buf.len() {
        match stream.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(got) => {
                n += got;
                let text = String::from_utf8_lossy(&buf[..n]);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let headers = &text[..header_end];
                    let need = content_length(headers).unwrap_or(0);
                    if n >= header_end + 4 + need || need > 8000 {
                        break;
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

fn content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn query_value(query: &str, key: &str) -> Option<String> {
    for part in query.split('&') {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        if name == key {
            return Some(percent_decode(value));
        }
    }
    None
}

fn form_value(body: &str, key: &str) -> String {
    query_value(body, key)
        .unwrap_or_default()
        .replace('+', " ")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[index + 1]), from_hex(bytes[index + 2]))
            {
                out.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' { b' ' } else { bytes[index] });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn portal_register(portal: &str, port: u16) -> Result<(), String> {
    let body = format!("slug={WEB_SLUG}&port={port}&title={}", form_enc(WEB_TITLE));
    http_post(portal, "/api/app-web/register", &body)
}

fn portal_unregister(portal: &str) -> Result<(), String> {
    http_post(portal, "/api/app-web/unregister", &format!("slug={WEB_SLUG}"))
}

fn http_post(base: &str, path: &str, body: &str) -> Result<(), String> {
    let base = base.trim_end_matches('/');
    let url = format!("{base}{path}");
    let (host, port) = parse_http(&url)?;
    let mut stream = TcpStream::connect((host.as_str(), port)).map_err(|err| err.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|err| err.to_string())?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|err| err.to_string())?;
    if resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        Err(resp.lines().next().unwrap_or("portal error").to_string())
    }
}

fn parse_http(url: &str) -> Result<(String, u16), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "portal URL must be http://".to_string())?;
    let (hostport, _) = rest.split_once('/').unwrap_or((rest, ""));
    if let Some((host, port)) = hostport.rsplit_once(':') {
        let port = port.parse().map_err(|_| "bad portal port".to_string())?;
        Ok((host.to_string(), port))
    } else {
        Ok((hostport.to_string(), 80))
    }
}

fn form_enc(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_u16(key: &str, default: u16) -> u16 {
    env_or(key, &default.to_string())
        .parse()
        .unwrap_or(default)
}
