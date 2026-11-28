//! The party HUD's tiles: who is in the party, what their Roblox client is doing, and what
//! colour their outline gets.
//!
//! Pure data, no egui and no sockets: `game_overlay` writes these tiles into the panels file,
//! `overlay_window` paints them, and `vm-tests` drives the same functions on the build host.
//! The statuses come from the party socket's existing `Status` frames - nothing here polls.

use std::time::{Duration, Instant};

use crate::party::{PartyMember, PartySnapshot, PartyStatus};

/// How long a member who vanished from the snapshot is still drawn, in red.
///
/// The host drops a peer from `Members` the moment its socket closes, which is exactly the
/// "crashed / lost connection" case. Keeping the tile for a few seconds is what makes that
/// visible instead of the tile silently disappearing mid-match.
pub const GRACE: Duration = Duration::from_secs(30);

/// Outline colours, 2 px around the 8 px-rounded square.
pub const GREEN: (u8, u8, u8) = (0x22, 0xC5, 0x5E);
pub const GREY: (u8, u8, u8) = (0x9C, 0xA3, 0xAF);
pub const RED: (u8, u8, u8) = (0xEF, 0x44, 0x44);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileStatus {
    /// In this exact server: same place and same job id.
    InServer,
    /// Launching, loading, or waiting in the lobby for this server.
    Loading,
    /// Vanished from the party's member list while it was still in a match.
    Disconnected,
}

impl TileStatus {
    /// What the panels file carries. The overlay only compares the string.
    pub fn key(self) -> &'static str {
        match self {
            TileStatus::InServer => "server",
            TileStatus::Loading => "loading",
            TileStatus::Disconnected => "offline",
        }
    }

    pub fn from_key(raw: &str) -> Self {
        match raw {
            "server" => TileStatus::InServer,
            "offline" => TileStatus::Disconnected,
            _ => TileStatus::Loading,
        }
    }

    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            TileStatus::InServer => GREEN,
            TileStatus::Loading => GREY,
            TileStatus::Disconnected => RED,
        }
    }
}

/// One member tile, ready for the panels file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub id: u64,
    pub name: String,
    /// Up to two letters, drawn while the 60x60 headshot loads (and when it never arrives).
    pub initials: String,
    pub status: TileStatus,
    /// 60x60 headshot URL from the party member's identity. Empty when the party has none.
    pub headshot: String,
}


/// Up to two initials from a display name: `sarah the great` -> `ST`, `bob` -> `B`.
pub fn initials(name: &str) -> String {
    let mut out = String::with_capacity(2);
    for word in name.split_whitespace() {
        let Some(ch) = word.chars().find(|c| c.is_alphanumeric()) else {
            continue;
        };
        out.extend(ch.to_uppercase());
        if out.chars().count() == 2 {
            break;
        }
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

/// What colour a member's outline gets.
///
/// `InGame` only counts as "in your server" when the place *and* the job id match: the place
/// alone is the same game on any server, which is the mistake a green outline must not make.
pub fn status_for(
    status: PartyStatus,
    place_id: u64,
    job_id: &str,
    my_place: u64,
    my_job: &str,
) -> TileStatus {
    match status {
        PartyStatus::InGame
            if place_id != 0
                && place_id == my_place
                && !my_job.is_empty()
                && job_id == my_job =>
        {
            TileStatus::InServer
        }
        PartyStatus::Ready | PartyStatus::Launching | PartyStatus::InGame => TileStatus::Loading,
    }
}

/// The tiles for one snapshot, in the order the party lists its members.
pub fn tiles(
    snapshot: &PartySnapshot,
    my_place: u64,
    my_job: &str,
    me: Option<u64>,
) -> Vec<Tile> {
    snapshot
        .members
        .iter()
        .filter(|m| Some(m.user.id) == me || me.is_none() || m.user.id != 0)
        .map(|m| tile_for(m, my_place, my_job))
        .collect()
}

fn tile_for(member: &PartyMember, my_place: u64, my_job: &str) -> Tile {
    let name = member.user.label().to_string();
    Tile {
        id: member.user.id,
        initials: initials(&name),
        name,
        status: status_for(
            member.status,
            member.place_id,
            &member.job_id,
            my_place,
            my_job,
        ),
        headshot: member.user.headshot_url.clone(),
    }
}

/// Keeps the last few seconds of a member who left the party's list without saying goodbye.
///
/// Event driven: the only thing that ever moves a tile is a new snapshot (or the local server
/// changing), and the grace expiry is read from the clock when the tiles are rebuilt. There is
/// no timer and nothing wakes up on its own, so an idle party costs nothing.
#[derive(Debug, Default)]
pub struct Tracker {
    /// Tiles from the previous snapshot with the instant they were last seen.
    seen: Vec<(Tile, Instant)>,
    /// The party this tracker is describing; leaving clears everything.
    code: String,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the tiles: members from `snapshot`, plus any member who vanished within
    /// [`GRACE`] as `Disconnected`. `None` (left the party) drops every tile at once, which
    /// is what keeps the cached headshots down to the people who are actually in the party.
    pub fn update(
        &mut self,
        snapshot: Option<&PartySnapshot>,
        my_place: u64,
        my_job: &str,
        me: Option<u64>,
        now: Instant,
    ) -> Vec<Tile> {
        let Some(snapshot) = snapshot else {
            self.seen.clear();
            self.code.clear();
            return Vec::new();
        };
        if snapshot.code != self.code {
            // A different room: the previous party's members are not "disconnected".
            self.seen.clear();
            self.code = snapshot.code.clone();
        }
        let present = tiles(snapshot, my_place, my_job, me);
        let mut out: Vec<Tile> = Vec::with_capacity(present.len() + self.seen.len());
        let mut next: Vec<(Tile, Instant)> = Vec::with_capacity(out.capacity());
        for tile in &present {
            out.push(tile.clone());
            next.push((tile.clone(), now));
        }
        for (tile, at) in &self.seen {
            if present.iter().any(|p| p.id == tile.id) {
                continue;
            }
            // The instant is when the member was last actually seen, so a member who never
            // comes back still expires: refreshing it here would keep the red tile forever.
            if now.saturating_duration_since(*at) < GRACE {
                let mut gone = tile.clone();
                gone.status = TileStatus::Disconnected;
                out.push(gone.clone());
                next.push((gone, *at));
            }
        }
        self.seen = next;
        out
    }
}
