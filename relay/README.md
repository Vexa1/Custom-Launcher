# CREBLOX party relay

A stateless, zero-database relay for CREBLOX parties. One Cloudflare Worker plus one
Durable Object class: a room exists only while somebody is connected to it, and its state
is deleted the moment the last member leaves.

- **No accounts, no storage, no secrets.** The relay never sees a `.ROBLOSECURITY` cookie or
  a launch ticket — CREBLOX sends only a public user id, name, display name and headshot URL.
- **Codes are generated server-side** (`crypto.getRandomValues`, 6 characters from a
  32-symbol alphabet without `I/O/0/1`, so they can be read aloud).
- **Hibernatable sockets**, so an idle room costs nothing between messages.

## Deploy (about a minute)

```bash
cd relay
npx wrangler deploy
```

Wrangler prints the deployment URL, for example:

```
https://creblox-party.<your-subdomain>.workers.dev
```

The party endpoint is that URL plus `/party`:

```
wss://creblox-party.<your-subdomain>.workers.dev/party
```

Put it in `~/Library/Application Support/CREBLOX/settings.json` (or let the UI write it):

```json
{ "party_relay_url": "wss://creblox-party.<your-subdomain>.workers.dev/party" }
```

With `party_relay_url` empty, CREBLOX uses the built-in default in `src/party.rs`
(`DEFAULT_RELAY_URL`). Point that constant at your own deployment before shipping builds you
expect to work out of the box.

## Protocol

Connect with `?create=1` to allocate a room, or `?code=CRB-8X2K` to join one.

Client → relay

| `t` | fields | meaning |
|---|---|---|
| `create` | `user`, `max` | open the room just allocated for this socket |
| `join` | `code`, `user`, `max` | enter an existing room (also used on reconnect) |
| `status` | `status` (`ready`/`launching`/`ingame`), `place_id`, `job_id` | member presence |
| `launch` | `place_id`, `job_id`, `universe_id`, `game_name` | **host only**: everyone join this server |
| `kick` | `user_id` | **host only** |
| `promote` | `user_id` | **host only** |
| `leave` | — | leave and close |
| `ping` | — | heartbeat, answered with `pong` |

Relay → client

| `t` | fields |
|---|---|
| `welcome` | `code`, `host`, `max` |
| `members` | `host`, `max`, `members[{user, status, place_id, job_id}]` |
| `launch` | `place_id`, `job_id`, `universe_id`, `game_name`, `by` |
| `error` | `message` |
| `pong` | — |

Everything is one JSON object per WebSocket frame.

## Limits and behaviour

- Up to 12 members per room (the client offers 8); the host is the first member and is
  reassigned to the next member if the host disconnects.
- A room with nobody left is deleted immediately, which is also what makes the codes
  disposable: they are not reserved, so they cannot be squatted.
- Identity is **self-declared**. Treat a party as a convenience, not as an authentication
  boundary: it can coordinate a launch, it cannot prove who anyone is. That is deliberate —
  proving identity would mean sending Roblox credentials to a third party.
- An empty room costs nothing; a room with hibernated sockets costs nothing between frames.

## Local development

```bash
cd relay
npx wrangler dev --local
```

Then set `party_relay_url` to `ws://127.0.0.1:8787/party` and use the same binary: no code
changes are needed to talk to a local relay.
