// CREBLOX party relay — one Durable Object per party room, no database, no accounts.
//
// A room exists only while someone is in it: the Durable Object lives in memory, and the
// moment its last socket closes it clears its state and can be evicted. Party codes are
// generated here (never by the client), so two clients can never pick the same one.
//
// Transport
//   wss://<worker>/party?create=1        -> the relay allocates a code and replies `welcome`
//   wss://<worker>/party?code=CRB-8X2K   -> join a room
//
// Identity is self-declared (Roblox user id, name, headshot URL). This relay is a
// coordination channel, not an authority: it never sees a .ROBLOSECURITY cookie, a launch
// ticket, or any Roblox credential, and it stores nothing after the room empties.

const CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no I/O/0/1: readable out loud
const MAX_MEMBERS_DEFAULT = 8;
const MAX_MEMBERS_LIMIT = 12;

function newCode() {
  const bytes = new Uint8Array(6);
  crypto.getRandomValues(bytes);
  let out = "";
  for (const b of bytes) out += CODE_ALPHABET[b % CODE_ALPHABET.length];
  return "CRB-" + out;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/party") {
      return new Response("CREBLOX party relay\n", {
        headers: { "content-type": "text/plain; charset=utf-8" },
      });
    }
    if (request.headers.get("Upgrade") !== "websocket") {
      return new Response("expected a websocket upgrade", { status: 426 });
    }

    const creating = url.searchParams.get("create") === "1";
    const code = creating ? newCode() : (url.searchParams.get("code") || "").toUpperCase();
    if (!/^CRB-[A-Z0-9]{4,8}$/.test(code)) {
      return new Response("bad party code", { status: 400 });
    }

    // One Durable Object per code. `idFromName` is deterministic, so a reconnecting member
    // lands in the same room without any lookup table.
    const stub = env.ROOMS.get(env.ROOMS.idFromName(code));
    const headers = new Headers(request.headers);
    headers.set("x-room-code", code);
    headers.set("x-room-create", creating ? "1" : "0");
    return stub.fetch(new Request("https://room/", { headers }));
  },
};

export class PartyRoom {
  constructor(state, env) {
    this.state = state;
    // `members` survives hibernation; the sockets themselves are handled by the runtime.
    this.members = null; // Map<userId, member> loaded lazily from storage
    this.host = null;
    this.max = MAX_MEMBERS_DEFAULT;
  }

  async load() {
    if (this.members) return;
    const saved = await this.state.storage.get("room");
    if (saved) {
      this.members = new Map(saved.members);
      this.host = saved.host;
      this.max = saved.max;
    } else {
      this.members = new Map();
    }
  }

  async persist() {
    // Only a room that still has people in it is worth keeping. An empty room is deleted
    // outright: party state is ephemeral by design.
    //
    // `members` itself stays an empty Map: nulling it here used to make the caller (drop)
    // read `.size` of null, so the last member leaving a room threw and their socket was
    // never closed. An empty Map is also what `load()` treats as authoritative for this
    // instance, which is correct after the storage delete.
    if (this.members.size === 0) {
      await this.state.storage.deleteAll();
      this.host = null;
      this.max = MAX_MEMBERS_DEFAULT;
      return;
    }
    await this.state.storage.put("room", {
      host: this.host,
      max: this.max,
      members: [...this.members.entries()],
    });
  }

  async fetch(request) {
    await this.load();
    const code = request.headers.get("x-room-code") || "";
    const creating = request.headers.get("x-room-create") === "1";

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    // Hibernatable socket: the room costs nothing while nobody is talking.
    this.state.acceptWebSocket(server);
    server.serializeAttachment({ code, creating });
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, raw) {
    await this.load();
    let msg;
    try {
      msg = JSON.parse(typeof raw === "string" ? raw : new TextDecoder().decode(raw));
    } catch {
      return this.send(ws, { t: "error", message: "bad message" });
    }

    const att = ws.deserializeAttachment() || {};
    const code = att.code || "";

    switch (msg.t) {
      case "ping":
        return this.send(ws, { t: "pong" });

      case "create":
      case "join":
        return this.join(ws, code, msg);

      case "status": {
        const me = this.bySocket(ws);
        if (!me) return;
        me.status = ["ready", "launching", "ingame"].includes(msg.status)
          ? msg.status
          : "ready";
        me.place_id = Number(msg.place_id) || 0;
        me.job_id = typeof msg.job_id === "string" ? msg.job_id.slice(0, 64) : "";
        await this.persist();
        return this.broadcast();
      }

      case "launch": {
        const me = this.bySocket(ws);
        // Only the host may move the party, and only to a real place.
        if (!me || me.user.id !== this.host) {
          return this.send(ws, { t: "error", message: "Only the host can start a party launch" });
        }
        const launch = {
          t: "launch",
          place_id: Number(msg.place_id) || 0,
          job_id: typeof msg.job_id === "string" ? msg.job_id.slice(0, 64) : "",
          universe_id: Number(msg.universe_id) || 0,
          game_name: typeof msg.game_name === "string" ? msg.game_name.slice(0, 120) : "",
          by: me.user.id,
        };
        if (!launch.place_id) {
          return this.send(ws, { t: "error", message: "Launch needs a place" });
        }
        return this.broadcast(launch);
      }

      case "kick": {
        const me = this.bySocket(ws);
        if (!me || me.user.id !== this.host) {
          return this.send(ws, { t: "error", message: "Only the host can remove members" });
        }
        const target = Number(msg.user_id) || 0;
        if (target === this.host) return;
        const victim = this.members.get(target);
        if (!victim) return;
        this.members.delete(target);
        // Match on the socket's own attachment, not through `members`: the victim was just
        // removed from it, so looking them up there found nobody and the kicked client kept
        // its socket open.
        for (const s of this.state.getWebSockets()) {
          const attached = s.deserializeAttachment() || {};
          if (attached.id === target) {
            try {
              s.close(4001, "removed from party");
            } catch {}
          }
        }
        await this.persist();
        return this.broadcast();
      }

      case "promote": {
        const me = this.bySocket(ws);
        if (!me || me.user.id !== this.host) {
          return this.send(ws, { t: "error", message: "Only the host can transfer the party" });
        }
        const target = Number(msg.user_id) || 0;
        if (this.members.has(target)) this.host = target;
        await this.persist();
        return this.broadcast();
      }

      case "leave":
        return this.drop(ws, "left");

      default:
        return this.send(ws, { t: "error", message: "unknown message" });
    }
  }

  async join(ws, code, msg) {
    const user = msg.user || {};
    const id = Number(user.id) || 0;
    if (!id || !user.name) {
      return this.send(ws, { t: "error", message: "A signed-in Roblox account is required" });
    }
    // A reconnect of the same member replaces the old socket instead of adding a second row.
    const existing = this.members.get(id);
    if (!existing && this.members.size >= this.max) {
      return this.send(ws, { t: "error", message: "This party is full" });
    }
    const member = {
      user: {
        id,
        name: String(user.name).slice(0, 32),
        display_name: String(user.display_name || user.name).slice(0, 32),
        headshot_url: String(user.headshot_url || "").slice(0, 300),
      },
      status: existing?.status || "ready",
      place_id: existing?.place_id || 0,
      job_id: existing?.job_id || "",
    };
    this.members.set(id, member);
    if (!this.host) this.host = id;
    if (Number(msg.max) > 0) {
      this.max = Math.min(MAX_MEMBERS_LIMIT, Math.max(2, Number(msg.max)));
    }
    ws.serializeAttachment({ code, id });
    await this.persist();
    this.send(ws, { t: "welcome", code, host: this.host, max: this.max });
    this.broadcast();
  }

  async drop(ws, reason) {
    const me = this.bySocket(ws);
    if (!me) return;
    this.members.delete(me.user.id);
    // The party keeps its host while the host is present; otherwise the oldest member
    // becomes the host, so a room can never end up with nobody able to launch.
    if (this.host === me.user.id) {
      this.host = this.members.keys().next().value ?? null;
    }
    // Decide emptiness before persisting: `persist` may clear the storage, and the answer
    // must not depend on a field it touched.
    const empty = this.members.size === 0;
    await this.persist();
    if (empty) {
      try {
        ws.close(1000, reason);
      } catch {}
      return;
    }
    this.broadcast();
  }

  bySocket(ws) {
    const id = (ws.deserializeAttachment() || {}).id;
    return id ? this.members.get(id) : undefined;
  }

  send(ws, obj) {
    try {
      ws.send(JSON.stringify(obj));
    } catch {}
  }

  broadcast(extra) {
    // An empty room has nobody to tell, and building the frame would walk a map that may
    // have just been cleared.
    if (!extra && this.members.size === 0) return undefined;
    const frame = extra
      ? extra
      : {
          t: "members",
          host: this.host,
          max: this.max,
          members: [...this.members.values()],
        };
    const data = JSON.stringify(frame);
    for (const ws of this.state.getWebSockets()) {
      try {
        ws.send(data);
      } catch {}
    }
    return undefined;
  }

  async webSocketClose(ws, code, reason) {
    await this.load();
    await this.drop(ws, reason || "closed");
  }

  async webSocketError(ws) {
    await this.load();
    await this.drop(ws, "error");
  }
}
