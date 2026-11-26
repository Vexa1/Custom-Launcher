// End-to-end exercise of the party relay over real WebSockets.
//
//   cd relay && npx wrangler dev --local --port 8787 &
//   node test/party-test.mjs
//
// Covers: create, join, presence, host-only rules, launch broadcast, promote, kick,
// reconnect, "party is full", and the room disappearing when the last member leaves.

const BASE = process.env.PARTY_RELAY || "ws://127.0.0.1:8787/party";
let failures = 0;

function check(name, condition, detail) {
  if (condition) {
    console.log(`  PASS  ${name}`);
  } else {
    failures += 1;
    console.log(`  FAIL  ${name}${detail ? ` -- ${detail}` : ""}`);
  }
}

class Peer {
  constructor(ws) {
    this.ws = ws;
    this.queue = [];
    this.waiters = [];
    this.closed = null;
    ws.addEventListener("message", (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      this.queue.push(msg);
      this.flush();
    });
    ws.addEventListener("close", (ev) => {
      this.closed = { code: ev.code, reason: ev.reason };
      this.flush();
    });
  }

  flush() {
    for (let i = this.waiters.length - 1; i >= 0; i -= 1) {
      const waiter = this.waiters[i];
      const at = this.queue.findIndex(waiter.pred);
      if (at >= 0) {
        const [msg] = this.queue.splice(at, 1);
        this.waiters.splice(i, 1);
        clearTimeout(waiter.timer);
        waiter.resolve(msg);
      }
    }
  }

  /// Waits for the first frame matching `pred`, skipping older frames. Roster frames are
  /// broadcast on every change, so "the next members frame" is rarely the current state.
  nextWhere(pred, label = "frame", timeout = 5000) {
    const at = this.queue.findIndex(pred);
    if (at >= 0) return Promise.resolve(this.queue.splice(at, 1)[0]);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const seen = JSON.stringify(this.queue);
        this.waiters = this.waiters.filter((w) => w.pred !== pred);
        reject(new Error(`timed out waiting for ${label}; queued: ${seen}`));
      }, timeout);
      const waiter = { pred, timer };
      waiter.resolve = (msg) => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        resolve(msg);
      };
      this.waiters.push(waiter);
    });
  }

  next(type, timeout = 5000) {
    return this.nextWhere((m) => m.t === type, `"${type}"`, timeout);
  }

  /// The roster frame that shows `count` members (or the next one, if none matches yet).
  roster(count, timeout = 5000) {
    return this.nextWhere(
      (m) => m.t === "members" && (!count || m.members.length === count),
      `roster of ${count ?? "any"}`,
      timeout,
    );
  }

  drain() {
    const out = this.queue.slice();
    this.queue.length = 0;
    return out;
  }

  send(obj) {
    this.ws.send(JSON.stringify(obj));
  }

  async close() {
    this.ws.close();
  }

  waitClosed(timeout = 5000) {
    if (this.closed) return Promise.resolve(this.closed);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("socket did not close")), timeout);
      const poll = setInterval(() => {
        if (this.closed) {
          clearInterval(poll);
          clearTimeout(timer);
          resolve(this.closed);
        }
      }, 25);
    });
  }
}

async function connect(query) {
  const ws = new WebSocket(`${BASE}?${query}`);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", () => reject(new Error(`could not open ${query}`)), {
      once: true,
    });
  });
  return new Peer(ws);
}

const user = (id, name) => ({
  id,
  name,
  display_name: name.toUpperCase(),
  headshot_url: `https://example.invalid/${id}.png`,
});

async function main() {
  console.log(`relay: ${BASE}`);

  // --- create -------------------------------------------------------------
  const host = await connect("create=1");
  host.send({ t: "create", user: user(1, "host"), max: 8 });
  const welcome = await host.next("welcome");
  check("create returns a room code", /^CRB-[A-Z0-9]{6}$/.test(welcome.code), welcome.code);
  check("creator is the host", welcome.host === 1, JSON.stringify(welcome));
  const code = welcome.code;

  // --- join ---------------------------------------------------------------
  const guest = await connect(`code=${code}`);
  guest.send({ t: "join", code, user: user(2, "guest"), max: 8 });
  const guestWelcome = await guest.next("welcome");
  check("join gets a welcome", guestWelcome.code === code, JSON.stringify(guestWelcome));
  check("join keeps the existing host", guestWelcome.host === 1);

  const hostRoster = await host.roster(2);
  check("host is told about the new member", hostRoster.members.length === 2, JSON.stringify(hostRoster));
  const guestRoster = await guest.roster(2);
  check("guest receives the roster", guestRoster.members.length === 2);

  // --- presence -----------------------------------------------------------
  guest.send({ t: "status", status: "ingame", place_id: 1818, job_id: "job-guest" });
  const afterStatus = await host.nextWhere(
    (m) => m.t === "members" && m.members.some((row) => row.user.id === 2 && row.status === "ingame"),
    "guest marked as in game",
  );
  const guestRow = afterStatus.members.find((m) => m.user.id === 2);
  check("status reaches the other member", guestRow?.status === "ingame", JSON.stringify(guestRow));
  check("place and job are carried", guestRow?.place_id === 1818 && guestRow?.job_id === "job-guest");

  // --- heartbeat ----------------------------------------------------------
  guest.send({ t: "ping" });
  const pong = await guest.next("pong");
  check("ping is answered with pong", pong.t === "pong");

  // --- host-only rules ----------------------------------------------------
  guest.send({ t: "launch", place_id: 1818, job_id: "job-rogue" });
  const guestError = await guest.next("error");
  check("members cannot launch the party", /host/i.test(guestError.message), guestError.message);
  guest.send({ t: "kick", user_id: 1 });
  const kickError = await guest.next("error");
  check("members cannot kick", /host/i.test(kickError.message), kickError.message);

  // --- launch broadcast ---------------------------------------------------
  host.send({
    t: "launch",
    place_id: 1818,
    job_id: "job-shared",
    universe_id: 99,
    game_name: "Test Place",
  });
  const hostLaunch = await host.next("launch");
  const guestLaunch = await guest.next("launch");
  check("host receives its own launch", hostLaunch.job_id === "job-shared", JSON.stringify(hostLaunch));
  check(
    "every member receives the launch",
    guestLaunch.job_id === "job-shared" &&
      guestLaunch.place_id === 1818 &&
      guestLaunch.game_name === "Test Place" &&
      guestLaunch.by === 1,
    JSON.stringify(guestLaunch),
  );

  // --- promote ------------------------------------------------------------
  host.send({ t: "promote", user_id: 2 });
  const promoted = await guest.nextWhere((m) => m.t === "members" && m.host === 2, "promotion");
  check("host can be transferred", promoted.host === 2, JSON.stringify(promoted));

  // --- kick ---------------------------------------------------------------
  guest.send({ t: "kick", user_id: 1 });
  const kickedClose = await host.waitClosed();
  check("the kicked socket is closed with 4001", kickedClose.code === 4001, JSON.stringify(kickedClose));
  const afterKick = await guest.roster(1);
  check("the kicked member is gone from the roster", afterKick.members.length === 1, JSON.stringify(afterKick));

  // --- explicit leave -----------------------------------------------------
  const leaver = await connect(`code=${code}`);
  leaver.send({ t: "join", code, user: user(3, "leaver"), max: 8 });
  await leaver.next("welcome");
  await guest.roster(2);
  leaver.send({ t: "leave" });
  const afterLeave = await guest.roster(1);
  check(
    "leave removes the member from the roster",
    afterLeave.members.length === 1 && afterLeave.members[0].user.id === 2,
    JSON.stringify(afterLeave),
  );
  await leaver.close();

  // --- reconnect ----------------------------------------------------------
  const rejoined = await connect(`code=${code}`);
  rejoined.send({ t: "join", code, user: user(1, "host"), max: 8 });
  const back = await rejoined.next("welcome");
  check("a reconnecting member lands in the same room", back.code === code, JSON.stringify(back));
  check("the promoted host is still host", back.host === 2, JSON.stringify(back));
  const rosterBack = await rejoined.next("members");
  check("the room still holds both members", rosterBack.members.length === 2, JSON.stringify(rosterBack));

  // --- full room ----------------------------------------------------------
  const owner = await connect("create=1");
  owner.send({ t: "create", user: user(10, "owner"), max: 2 });
  const smallRoom = await owner.next("welcome");
  const second = await connect(`code=${smallRoom.code}`);
  second.send({ t: "join", code: smallRoom.code, user: user(11, "second"), max: 2 });
  await second.next("welcome");
  const third = await connect(`code=${smallRoom.code}`);
  third.send({ t: "join", code: smallRoom.code, user: user(12, "third"), max: 2 });
  const fullError = await third.next("error");
  check("a third member is refused by a 2-slot room", /full/i.test(fullError.message), fullError.message);

  // --- room disappears when empty ----------------------------------------
  await rejoined.close();
  await guest.close();
  await new Promise((r) => setTimeout(r, 300));
  const late = await connect(`code=${code}`);
  late.send({ t: "join", code, user: user(1, "host"), max: 8 });
  const lateWelcome = await late.next("welcome");
  check("an emptied room becomes a fresh one", lateWelcome.host === 1, JSON.stringify(lateWelcome));
  const lateRoster = await late.next("members");
  check("the fresh room has one member", lateRoster.members.length === 1, JSON.stringify(lateRoster));

  await late.close();
  await owner.close();
  await second.close();
  await third.close();
  await new Promise((r) => setTimeout(r, 200));

  console.log(failures === 0 ? "\nall party relay checks passed" : `\n${failures} check(s) failed`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error(`\nrelay test crashed: ${e.stack || e}`);
  process.exit(1);
});
