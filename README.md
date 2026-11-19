# pnet_messages

One person writes to another. The server stores every message and is the part that sends it on.

`messages-server` runs on server-grade nodes. The top-ranked one opens `~/.pnet/messages/messages` and mounts `/apps/messages/` on that node's owner portal. The page lists this user's contacts. A send is stored here, then delivered to that contact's `messages-server`. A server that is not the top-ranked one stands by and does not open a second file.

| | |
|---|---|
| Protocol | `application/messages` |
| Server push | `127.0.0.1:8831` (`MESSAGES_PUSH_PORT`) |
| Web interface | `127.0.0.1:8832` (`MESSAGES_HTTP_PORT`), portal slug `messages` |
| Store | `~/.pnet/messages` (`MESSAGES_DIR`) |
| Local node | `PNET_ADDR`, default `127.0.0.1:7777` |
| Portal | `PNET_PORTAL`, default `http://127.0.0.1:8777` |

```bash
cargo run --bin messages-server
```

Open Messages at `/apps/messages/`.
