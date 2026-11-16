# pnet_messages

One person writes to another. The client shows the conversation. The server stores every message and is the part that sends it on.

`messages-client` runs on each device. It mounts `/apps/messages/` on that device's owner portal and keeps no message file. A send goes to this user's own `messages-server`.

`messages-server` runs on server-grade nodes. The top-ranked one opens `~/.pnet/messages/messages` and answers the clients. When a message is new, that server sends it to the other person's `messages-server`, which stores it for that person's clients. A server that is not the top-ranked one stands by and does not open a second file.

Approve both apps on the portal before they will talk. The page follows the portal light and dark theme (`pnet-theme`, dark by default) and does not draw its own toggle.

| | |
|---|---|
| Protocol | `application/messages` |
| Client push | `127.0.0.1:8830` (`MESSAGES_PUSH_PORT`) |
| Server push | `127.0.0.1:8831` (`MESSAGES_PUSH_PORT`) |
| Client page | `127.0.0.1:8832` (`MESSAGES_HTTP_PORT`), portal slug `messages` |
| Store | `~/.pnet/messages` (`MESSAGES_DIR`) |
| Local node | `PNET_ADDR`, default `127.0.0.1:7777` |
| Portal | `PNET_PORTAL`, default `http://127.0.0.1:8777` |

```bash
cargo run --bin messages-server
cargo run --bin messages-client
```

Open Messages at `/apps/messages/`.
