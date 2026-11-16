//! Shows this user's messages. The page asks the user's server each time.
//! Nothing in this process is written except the app token.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pnet_messages::directory::{self, Directory, Peer};
use pnet_messages::node::{self, NodeClient};
use pnet_messages::page::{self, Person};
use pnet_messages::proto::{self, Frame, Post};
use pnet_messages::store::{self, Message};
use pnet_messages::{CLIENT_ALIAS, DEFAULT_CLIENT_PUSH, DEFAULT_HTTP_PORT, WEB_SLUG, WEB_TITLE};

struct Session {
    node: NodeClient,
    push: std::net::UdpSocket,
    token: [u8; 16],
}

fn main() {
    pnet_messages::log_setup::init("messages-client");
    let addr = env_or("PNET_ADDR", "127.0.0.1:7777");
    let addr = addr.parse().unwrap_or_else(|_| {
        log::error!("[messages-client] PNET_ADDR is not host:port");
        std::process::exit(1);
    });
    let push_port = env_u16("MESSAGES_PUSH_PORT", DEFAULT_CLIENT_PUSH);
    let http_port = env_u16("MESSAGES_HTTP_PORT", DEFAULT_HTTP_PORT);
    let portal = env_or("PNET_PORTAL", "http://127.0.0.1:8777");

    let push = node::bind_push(push_port).unwrap_or_else(|err| {
        log::error!("[messages-client] bind 127.0.0.1:{push_port}: {err}");
        std::process::exit(1);
    });
    let node = NodeClient::connect(addr).unwrap_or_else(|err| {
        log::error!("[messages-client] local node {addr}: {err}");
        std::process::exit(1);
    });
    let token = node.register(CLIENT_ALIAS, push_port).unwrap_or_else(|err| {
        log::error!("[messages-client] register failed: {err}");
        std::process::exit(1);
    });
    if let Err(err) = node::save_token(&node::token_path("messages-client"), &token) {
        log::error!("[messages-client] token file: {err}");
    }
    log::info!("[messages-client] registered as {CLIENT_ALIAS}");

    let listener = TcpListener::bind(("127.0.0.1", http_port)).unwrap_or_else(|err| {
        log::error!("[messages-client] http 127.0.0.1:{http_port}: {err}");
        std::process::exit(1);
    });
    listener.set_nonblocking(true).ok();
    let session = Arc::new(Mutex::new(Session { node, push, token }));
    let mounted = Arc::new(AtomicBool::new(false));
    {
        let mounted = Arc::clone(&mounted);
        let portal = portal.clone();
        thread::spawn(move || {
            for attempt in 1..=8 {
                match portal_register(&portal, http_port) {
                    Ok(()) => {
                        log::info!(
                            "[messages-client] portal /apps/{WEB_SLUG}/ → 127.0.0.1:{http_port}"
                        );
                        mounted.store(true, Ordering::SeqCst);
                        return;
                    }
                    Err(err) => {
                        log::error!("[messages-client] portal register attempt {attempt}: {err}");
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        });
    }

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let session = Arc::clone(&session);
                thread::spawn(move || {
                    if let Err(err) = handle(stream, &session) {
                        log::error!("[messages-client] http: {err}");
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(40));
            }
            Err(err) => {
                log::error!("[messages-client] accept: {err}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle(mut stream: TcpStream, session: &Mutex<Session>) -> Result<(), String> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let req = read_request(&mut stream)?;
    let first = req.lines().next().unwrap_or("");
    if first.contains("favicon") {
        stream.write_all(page::http_response("404 Not Found", "text/plain", "").as_slice()).ok();
        return Ok(());
    }
    let query = first.split_whitespace().nth(1).unwrap_or("/");
    let query = query.split_once('?').map(|(_, q)| q).unwrap_or("");
    let with = query_value(query, "with").and_then(|text| store::parse_hex_id(&text));

    let mut guard = session.lock().unwrap_or_else(|err| err.into_inner());
    let dir = match fetch(&guard.node, &guard.token) {
        Ok(dir) => dir,
        Err(err) => {
            let html = page::render("", &[], None, &[], &err);
            stream.write_all(&page::http_response("200 OK", "text/html; charset=utf-8", &html)).ok();
            return Ok(());
        }
    };
    let Some(server) = directory::own_server(&dir) else {
        let people = people_of(&dir, &[]);
        let open = with.and_then(|id| people.iter().find(|person| person.id == id)).cloned();
        let html = page::render(
            &dir.owner_alias,
            &people,
            open.as_ref(),
            &[],
            "The server is not visible yet.",
        );
        stream.write_all(&page::http_response("200 OK", "text/html; charset=utf-8", &html)).ok();
        return Ok(());
    };

    if first.starts_with("POST ") {
        let body = req.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or("");
        let peer = store::parse_hex_id(&form_value(body, "with")).or(with);
        let text = form_value(body, "text");
        let id = fresh_id();
        if let Some(peer_id) = peer {
            if proto::is_text(&text) {
                let payload = proto::encode(&Frame::Post(Post {
                    id,
                    to_user: peer_id,
                    text,
                }));
                if let Some(payload) = payload {
                    let _ = ask(&mut guard, &server, &payload, |frame| {
                        matches!(frame, Frame::PostAck { id: ack, .. } if *ack == id)
                    });
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

    let threads = match ask(
        &mut guard,
        &server,
        &proto::encode(&Frame::ThreadsReq).unwrap_or_default(),
        |frame| matches!(frame, Frame::ThreadsResp(_)),
    ) {
        Some(Frame::ThreadsResp(rows)) => rows,
        _ => Vec::new(),
    };
    let people = people_of(&dir, &threads);
    let open = with
        .and_then(|id| people.iter().find(|person| person.id == id))
        .cloned();
    let messages = if let Some(person) = &open {
        history(&mut guard, &server, person.id)
    } else {
        Vec::new()
    };
    let lines = open
        .as_ref()
        .map(|person| page::lines_for(&dir.owner_alias, &person.alias, &dir.owner_id, &messages))
        .unwrap_or_default();
    let html = page::render(&dir.owner_alias, &people, open.as_ref(), &lines, "");
    stream
        .write_all(&page::http_response("200 OK", "text/html; charset=utf-8", &html))
        .map_err(|err| err.to_string())
}

fn history(session: &mut Session, server: &Peer, peer: [u8; 16]) -> Vec<Message> {
    let mut after = [0u8; 16];
    let mut all = Vec::new();
    for _ in 0..40 {
        let payload = proto::encode(&Frame::HistoryReq { peer, after }).unwrap_or_default();
        let Some(Frame::HistoryResp {
            messages, more, ..
        }) = ask(session, server, &payload, |frame| {
            matches!(frame, Frame::HistoryResp { peer: got, .. } if *got == peer)
        })
        else {
            break;
        };
        if messages.is_empty() {
            break;
        }
        after = messages.last().map(|message| message.id).unwrap_or(after);
        all.extend(messages);
        if !more {
            break;
        }
    }
    all
}

fn ask(
    session: &mut Session,
    server: &Peer,
    payload: &[u8],
    want: impl Fn(&Frame) -> bool,
) -> Option<Frame> {
    if session
        .node
        .send(&session.token, &server.device, &server.app, payload)
        .is_err()
    {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match node::recv_push(&session.push) {
            Ok(Some(datagram)) => {
                let Some((sender, body)) = node::decode_push(&datagram) else {
                    continue;
                };
                if sender != server.app {
                    continue;
                }
                if let Some(frame) = proto::decode(body) {
                    if want(&frame) {
                        return Some(frame);
                    }
                }
            }
            Ok(None) => {}
            Err(_) => break,
        }
    }
    None
}

fn people_of(dir: &Directory, threads: &[proto::ThreadRow]) -> Vec<Person> {
    let mut people: Vec<Person> = dir
        .contacts
        .iter()
        .map(|contact| Person {
            id: contact.id,
            alias: if contact.alias.is_empty() {
                store::hex(&contact.id[..4])
            } else {
                contact.alias.clone()
            },
            preview: String::new(),
            at_ms: 0,
        })
        .collect();
    for row in threads {
        let preview = row.preview.clone();
        let at_ms = row.at_ms;
        if let Some(person) = people.iter_mut().find(|person| person.id == row.peer) {
            person.preview = preview;
            person.at_ms = at_ms;
        } else {
            people.push(Person {
                id: row.peer,
                alias: directory::alias_for(dir, &row.peer),
                preview,
                at_ms,
            });
        }
    }
    people.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.alias.cmp(&b.alias)));
    people
}

fn fresh_id() -> [u8; 16] {
    let ms = pnet_messages::mail::now_ms();
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

fn fetch(node: &NodeClient, token: &[u8; 16]) -> Result<Directory, String> {
    let reply = node.get_data(token).map_err(|err| err.to_string())?;
    directory::parse_get_data(&reply).map_err(|err| err.to_string())
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
    query_value(body, key).unwrap_or_default().replace('+', " ")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[index + 1]), from_hex(bytes[index + 2])) {
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
    let body = format!("slug={WEB_SLUG}&port={port}&title={WEB_TITLE}");
    http_post(portal, "/api/app-web/register", &body)
}

fn http_post(base: &str, path: &str, body: &str) -> Result<(), String> {
    let base = base.trim_end_matches('/');
    let url = format!("{base}{path}");
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "portal URL must be http://".to_string())?;
    let (hostport, _) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = hostport
        .rsplit_once(':')
        .ok_or_else(|| "portal URL needs a port".to_string())?;
    let port: u16 = port.parse().map_err(|_| "bad portal port".to_string())?;
    let mut stream = TcpStream::connect((host, port)).map_err(|err| err.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).map_err(|err| err.to_string())?;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).map_err(|err| err.to_string())?;
    if resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        Err(resp.lines().next().unwrap_or("portal error").to_string())
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|value| !value.is_empty()).unwrap_or_else(|| default.to_string())
}

fn env_u16(key: &str, default: u16) -> u16 {
    env_or(key, &default.to_string()).parse().unwrap_or(default)
}
