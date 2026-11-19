//! The messages page. The storing server draws it from the message file and the contact list.

use crate::store::{self, Message};

#[derive(Clone)]
pub struct Person {
    pub id: [u8; 16],
    pub alias: String,
    pub preview: String,
    pub at_ms: u64,
}

pub struct Line {
    pub from_alias: String,
    pub mine: bool,
    pub at_ms: u64,
    pub text: String,
}

pub fn render(
    owner_alias: &str,
    people: &[Person],
    open: Option<&Person>,
    lines: &[Line],
    note: &str,
) -> String {
    let title = open
        .map(|person| person.alias.as_str())
        .unwrap_or("Messages");
    let mut body = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\" data-theme=\"dark\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"theme-color\" content=\"#070605\"><title>",
    );
    body.push_str(&esc(title));
    body.push_str(
        "</title>\n<script>\
         (function () {\
           var theme = \"dark\";\
           try {\
             var saved = localStorage.getItem(\"pnet-theme\");\
             if (saved === \"light\" || saved === \"dark\") theme = saved;\
           } catch (e) {}\
           document.documentElement.setAttribute(\"data-theme\", theme);\
           var meta = document.querySelector('meta[name=\"theme-color\"]');\
           if (meta) meta.setAttribute(\"content\", theme === \"light\" ? \"#f3eee4\" : \"#070605\");\
         })();\
         </script>\n<style>",
    );
    body.push_str(PAGE_CSS);
    body.push_str("</style></head><body>\n<div class=\"app\">\n<aside class=\"side\">\n");
    body.push_str("<div class=\"workspace\">");
    body.push_str(&esc(if owner_alias.is_empty() {
        "Messages"
    } else {
        owner_alias
    }));
    body.push_str("</div>\n");
    body.push_str(
        "<label class=\"search\"><input id=\"find\" type=\"search\" placeholder=\"Find a conversation...\" autocomplete=\"off\"></label>\n",
    );
    body.push_str("<div class=\"section\">Direct messages</div>\n<nav class=\"dms\">\n");
    if people.is_empty() {
        body.push_str("<p class=\"empty\">No contacts yet.</p>\n");
    }
    for person in people {
        let id = store::hex(&person.id);
        let selected = open.is_some_and(|open| open.id == person.id);
        body.push_str("<a class=\"dm");
        if selected {
            body.push_str(" on");
        }
        body.push_str("\" data-name=\"");
        body.push_str(&esc(&person.alias.to_lowercase()));
        body.push_str("\" href=\"?with=");
        body.push_str(&id);
        body.push_str("&amp;raw=1\"><span class=\"avatar\">");
        body.push_str(&esc(&initial(&person.alias)));
        body.push_str("</span><span class=\"who\"><span class=\"name\">");
        body.push_str(&esc(&person.alias));
        body.push_str("</span><span class=\"preview\">");
        body.push_str(&esc(if person.preview.is_empty() {
            "No messages yet"
        } else {
            person.preview.as_str()
        }));
        body.push_str("</span></span></a>\n");
    }
    body.push_str("</nav></aside>\n<main class=\"thread\">\n");
    match open {
        None => {
            body.push_str(
                "<header class=\"bar\"><h1>Messages</h1></header>\n\
                 <div class=\"stream\"><p class=\"hint\">Choose someone on the left.</p></div>\n",
            );
        }
        Some(person) => {
            let id = store::hex(&person.id);
            body.push_str("<header class=\"bar\"><span class=\"avatar\">");
            body.push_str(&esc(&initial(&person.alias)));
            body.push_str("</span><h1>");
            body.push_str(&esc(&person.alias));
            body.push_str("</h1></header>\n<div class=\"stream\" id=\"stream\">\n");
            if lines.is_empty() {
                body.push_str("<p class=\"hint\">This is the start of the conversation.</p>\n");
            }
            for line in lines {
                body.push_str("<article class=\"msg\"><span class=\"avatar\">");
                body.push_str(&esc(&initial(&line.from_alias)));
                body.push_str("</span><div><div class=\"meta\"><span class=\"name\">");
                body.push_str(&esc(&line.from_alias));
                body.push_str("</span> <time data-ms=\"");
                body.push_str(&line.at_ms.to_string());
                body.push_str("\"></time></div><p>");
                body.push_str(&esc(&line.text));
                body.push_str("</p></div></article>\n");
            }
            body.push_str("</div>\n<form class=\"composer\" method=\"post\" action=\"?raw=1\">\n");
            body.push_str("<input type=\"hidden\" name=\"with\" value=\"");
            body.push_str(&id);
            body.push_str("\">\n<textarea name=\"text\" rows=\"2\" maxlength=\"800\" placeholder=\"Message ");
            body.push_str(&esc(&person.alias));
            body.push_str("\" required></textarea>\n<button type=\"submit\">Send</button>\n</form>\n");
        }
    }
    if !note.is_empty() {
        body.push_str("<p class=\"note\">");
        body.push_str(&esc(note));
        body.push_str("</p>\n");
    }
    body.push_str(
        "</main></div>\n<script>\
         (function () {\
           var key = \"pnet-theme\";\
           function paint(theme) {\
             document.documentElement.setAttribute(\"data-theme\", theme);\
             var meta = document.querySelector('meta[name=\"theme-color\"]');\
             if (meta) meta.setAttribute(\"content\", theme === \"light\" ? \"#f3eee4\" : \"#070605\");\
           }\
           window.addEventListener(\"storage\", function (ev) {\
             if (ev.key === key && (ev.newValue === \"light\" || ev.newValue === \"dark\")) paint(ev.newValue);\
           });\
           var find = document.getElementById(\"find\");\
           if (find) find.addEventListener(\"input\", function () {\
             var q = find.value.trim().toLowerCase();\
             document.querySelectorAll(\".dm\").forEach(function (row) {\
               row.hidden = q.length > 0 && row.getAttribute(\"data-name\").indexOf(q) === -1;\
             });\
           });\
           var stream = document.getElementById(\"stream\");\
           var fmt = new Intl.DateTimeFormat(undefined, { hour: \"numeric\", minute: \"2-digit\" });\
           var day = new Intl.DateTimeFormat(undefined, { weekday: \"long\", month: \"long\", day: \"numeric\" });\
           function paintTimes(root) {\
             var last = \"\";\
             root.querySelectorAll(\".day\").forEach(function (sep) { sep.remove(); });\
             root.querySelectorAll(\"article.msg\").forEach(function (el) {\
               var time = el.querySelector(\"time\");\
               var ms = Number(time && time.getAttribute(\"data-ms\"));\
               if (!Number.isFinite(ms)) return;\
               var when = new Date(ms);\
               time.textContent = fmt.format(when);\
               var label = day.format(when);\
               if (label !== last) {\
                 var sep = document.createElement(\"div\");\
                 sep.className = \"day\";\
                 sep.textContent = label;\
                 el.parentNode.insertBefore(sep, el);\
                 last = label;\
               }\
             });\
           }\
           function stamp(root) {\
             return Array.prototype.map.call(root.querySelectorAll(\"article.msg\"), function (el) {\
               var time = el.querySelector(\"time\");\
               var text = el.querySelector(\"p\");\
               return (time ? time.getAttribute(\"data-ms\") : \"\") + \"\\n\" + (text ? text.textContent : \"\");\
             }).join(\"\\n\");\
           }\
           if (stream) {\
             paintTimes(stream);\
             stream.scrollTop = stream.scrollHeight;\
             setInterval(function () {\
               fetch(location.href, { cache: \"no-store\" }).then(function (res) { return res.text(); }).then(function (html) {\
                 var next = new DOMParser().parseFromString(html, \"text/html\").getElementById(\"stream\");\
                 if (!next || stamp(next) === stamp(stream)) return;\
                 var atBottom = stream.scrollHeight - stream.scrollTop - stream.clientHeight < 48;\
                 stream.innerHTML = next.innerHTML;\
                 paintTimes(stream);\
                 if (atBottom) stream.scrollTop = stream.scrollHeight;\
               }).catch(function () {});\
             }, 2000);\
           }\
           var box = document.querySelector(\"textarea\");\
           if (box) box.addEventListener(\"keydown\", function (ev) {\
             if (ev.key === \"Enter\" && !ev.shiftKey) { ev.preventDefault(); box.form.requestSubmit(); }\
           });\
         })();\
         </script>\n</body></html>\n",
    );
    body
}

pub fn http_response(status: &str, content_type: &str, body: &str) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut out = header.into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

pub fn lines_for(owner_alias: &str, peer_alias: &str, owner: &[u8; 16], messages: &[Message]) -> Vec<Line> {
    messages
        .iter()
        .map(|message| {
            let mine = message.from_user == *owner;
            Line {
                from_alias: if mine {
                    owner_alias.to_string()
                } else {
                    peer_alias.to_string()
                },
                mine,
                at_ms: message.at_ms,
                text: message.text.clone(),
            }
        })
        .collect()
}

fn initial(alias: &str) -> String {
    alias
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

const PAGE_CSS: &str = r#"
:root, html[data-theme="dark"] {
  color-scheme: dark;
  --bg: #070605; --side: #12100e; --elev: #1a1612;
  --ink: #f4efe6; --muted: #b7aa96; --gold: #d4b15a; --gold-ink: #1a1408;
  --line: rgba(244, 239, 230, 0.12); --hover: rgba(212, 177, 90, 0.12);
  --on: rgba(212, 177, 90, 0.22); --field: #100e0c;
}
html[data-theme="light"] {
  color-scheme: light;
  --bg: #f3eee4; --side: #fffdf8; --elev: #fffdf8;
  --ink: #241c14; --muted: #6b5844; --gold: #8d6818; --gold-ink: #fffdf8;
  --line: rgba(36, 28, 20, 0.12); --hover: rgba(141, 104, 24, 0.08);
  --on: rgba(141, 104, 24, 0.16); --field: #fffdf8;
}
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; }
body {
  background: var(--bg); color: var(--ink);
  font-family: "Segoe UI", system-ui, -apple-system, sans-serif;
}
.app { display: flex; height: 100%; min-height: 100%; }
.side {
  width: 260px; flex: 0 0 260px; background: var(--side);
  border-right: 1px solid var(--line); display: flex; flex-direction: column; min-height: 0;
}
.workspace {
  font-weight: 700; font-size: 1.05rem; padding: 0.85rem 1rem 0.4rem;
}
.search { display: block; padding: 0.35rem 0.75rem 0.6rem; }
.search input {
  width: 100%; border: 1px solid var(--line); background: var(--field); color: var(--ink);
  border-radius: 6px; padding: 0.4rem 0.6rem; font: inherit;
}
.section {
  color: var(--muted); font-size: 0.78rem; font-weight: 700; letter-spacing: 0.02em;
  padding: 0.45rem 1rem 0.25rem;
}
.dms { overflow: auto; flex: 1; padding-bottom: 0.6rem; }
.dm {
  display: flex; gap: 0.6rem; align-items: center; padding: 0.35rem 0.75rem;
  color: inherit; text-decoration: none; border-radius: 6px; margin: 0 0.4rem;
}
.dm:hover { background: var(--hover); }
.dm.on { background: var(--on); }
.avatar {
  width: 1.8rem; height: 1.8rem; border-radius: 6px; background: var(--gold); color: var(--gold-ink);
  display: inline-flex; align-items: center; justify-content: center; font-weight: 700; flex: 0 0 auto;
}
.who { min-width: 0; display: flex; flex-direction: column; }
.name { font-weight: 700; font-size: 0.92rem; }
.preview, .empty, .hint, .note { color: var(--muted); }
.preview { font-size: 0.78rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.thread { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; position: relative; }
.bar {
  display: flex; align-items: center; gap: 0.6rem; padding: 0.7rem 1rem;
  border-bottom: 1px solid var(--line); background: var(--bg);
}
.bar h1 { margin: 0; font-size: 1.05rem; font-weight: 700; }
.stream { flex: 1; overflow: auto; padding: 0.6rem 1.1rem 1rem; }
.day {
  display: flex; align-items: center; gap: 0.8rem; color: var(--muted); font-size: 0.78rem;
  margin: 0.8rem 0;
}
.day::before, .day::after { content: ""; flex: 1; height: 1px; background: var(--line); }
.msg { display: flex; gap: 0.7rem; padding: 0.35rem 0.4rem; border-radius: 6px; }
.msg:hover { background: var(--hover); }
.msg p { margin: 0.1rem 0 0; white-space: pre-wrap; word-break: break-word; }
.meta { font-size: 0.92rem; }
.meta time { color: var(--muted); font-size: 0.75rem; font-weight: 500; margin-left: 0.35rem; }
.composer {
  display: flex; gap: 0.5rem; align-items: flex-end; margin: 0 1rem 0.9rem;
  border: 1px solid var(--line); border-radius: 8px; padding: 0.45rem; background: var(--elev);
}
.composer textarea {
  flex: 1; resize: none; border: 0; background: transparent; color: var(--ink);
  font: inherit; min-height: 2.6rem; outline: none;
}
.composer button {
  background: var(--gold); color: var(--gold-ink); border: 0; border-radius: 6px;
  font-weight: 700; padding: 0.4rem 0.8rem; cursor: pointer;
}
.note { position: absolute; left: 1rem; bottom: 5.2rem; margin: 0; }
@media (max-width: 720px) {
  .side { width: 72px; flex-basis: 72px; }
  .workspace, .search, .section, .who { display: none; }
  .dm { justify-content: center; margin: 0.15rem; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_conversation_is_marked_and_the_theme_is_shared() {
        let chad = Person {
            id: [2; 16],
            alias: "chad".into(),
            preview: "hello".into(),
            at_ms: 50,
        };
        let html = render(
            "william",
            std::slice::from_ref(&chad),
            Some(&chad),
            &[Line {
                from_alias: "william".into(),
                mine: true,
                at_ms: 50,
                text: "hello <there>".into(),
            }],
            "",
        );
        assert!(html.contains("data-theme=\"dark\""));
        assert!(html.contains("pnet-theme"));
        assert!(html.contains("class=\"dm on\""));
        assert!(html.contains("Direct messages"));
        assert!(html.contains("hello &lt;there&gt;"));
        assert!(!html.contains("theme-toggle"));
        assert!(html.contains("raw=1"));
        assert!(html.contains("setInterval"));
        assert!(html.contains("getElementById(\"stream\")"));
    }
}
