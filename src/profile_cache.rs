//! The profile shelf: what the client already knows about a user, a bounded LRU of
//! recently loaded profiles, and the assembly of a profile that streams in one section
//! at a time.
//!
//! A profile page has no reason to start blank. Opening one from the friends strip, the
//! party bar, a search result or a server row already carries the user record - id, name,
//! display name, headshot, presence and the place they are in - so the header, the avatar
//! and the buttons can be drawn on the very first frame. Everything else (bio, counts,
//! wearing, badges) arrives later, section by section, and each section is either drawn or
//! replaced by a skeleton.
//!
//! No egui and no network here: `vm-tests` drives this module directly.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::types::{AvatarAsset, FriendProfile, Game, PlayerBadge, ProfileSections, User};

/// How many profiles stay in RAM. Each one holds a handful of games and assets, so 32 is
/// a few hundred kilobytes at worst and covers a full friends strip.
pub const CAPACITY: usize = 32;

/// How long the pointer has to rest on a friend's circle before their profile is warmed.
/// Long enough that sweeping across the strip starts nothing, short enough that a
/// deliberate hover has the profile ready before the click lands.
pub const HOVER_DWELL: Duration = Duration::from_millis(120);

/// How long a cached profile is served without asking Roblox again. Past this it is still
/// painted instantly, and refreshed in the background (stale-while-revalidate).
pub const TTL: Duration = Duration::from_secs(300);

/// Every section loaded: a profile that arrived end to end.
pub fn all_sections() -> ProfileSections {
    ProfileSections {
        info: true,
        friends: true,
        followers: true,
        following: true,
        avatar: true,
        wearing: true,
        badges: true,
        creations: true,
        favorites: true,
    }
}

/// True when nothing is still missing.
pub fn is_complete(sections: ProfileSections) -> bool {
    sections == all_sections()
}

/// A profile that contains only what the client already has in memory: the user record
/// (including headshot, presence and the place they are playing) and nothing else.
pub fn seed(user: &User) -> FriendProfile {
    FriendProfile {
        user: user.clone(),
        ..Default::default()
    }
}

/// One section of a profile, as it arrives from Roblox.
///
/// Every variant is independent: the loader sends each one the moment its own request
/// resolves, so a slow badges request never holds up the bio.
#[derive(Clone, Debug)]
pub enum ProfileUpdate {
    /// `users.roblox.com/v1/users/{id}`: bio, join date, verified badge, current names.
    Info {
        name: String,
        display: String,
        description: String,
        created: String,
        verified: bool,
    },
    Friends(u64),
    Followers(u64),
    Following(u64),
    /// `thumbnails.roblox.com/v1/users/avatar`.
    Avatar(String),
    /// `thumbnails.roblox.com/v1/users/avatar-headshot`, only for a seed without one.
    Headshot(String),
    /// Presence: refreshed after the seed, and it can reveal the place being played.
    Presence {
        presence: u8,
        location: String,
        place_id: Option<u64>,
    },
    Playing(Option<Game>),
    Wearing(Vec<AvatarAsset>),
    Badges(Vec<PlayerBadge>),
    Creations(Vec<Game>),
    Favorites(Vec<Game>),
    /// This section's request failed. Other sections still land.
    Failed(String),
    /// No more updates will arrive.
    Done,
}

/// Puts one update into a profile and marks that section as loaded.
pub fn apply(profile: &mut FriendProfile, update: ProfileUpdate) {
    match update {
        ProfileUpdate::Info {
            name,
            display,
            description,
            created,
            verified,
        } => {
            if !name.is_empty() {
                profile.user.name = name;
            }
            if !display.is_empty() {
                profile.user.display = display;
            }
            profile.description = description;
            profile.created = created;
            profile.verified = verified;
            profile.user.verified = verified;
            profile.sections.info = true;
        }
        ProfileUpdate::Friends(n) => {
            profile.friends = n;
            profile.sections.friends = true;
        }
        ProfileUpdate::Followers(n) => {
            profile.followers = n;
            profile.sections.followers = true;
        }
        ProfileUpdate::Following(n) => {
            profile.following = n;
            profile.sections.following = true;
        }
        ProfileUpdate::Avatar(url) => {
            if !url.is_empty() {
                profile.avatar = url;
            }
            profile.sections.avatar = true;
        }
        ProfileUpdate::Headshot(url) => {
            if !url.is_empty() {
                profile.user.headshot = url;
            }
        }
        ProfileUpdate::Presence {
            presence,
            location,
            place_id,
        } => {
            profile.user.presence = presence;
            profile.user.location = location;
            if place_id.is_some() {
                profile.user.place_id = place_id;
            }
        }
        ProfileUpdate::Playing(game) => {
            if game.is_some() {
                profile.playing_game = game;
            }
        }
        ProfileUpdate::Wearing(items) => {
            profile.currently_wearing = items;
            profile.sections.wearing = true;
        }
        ProfileUpdate::Badges(items) => {
            profile.badges = items;
            profile.sections.badges = true;
        }
        ProfileUpdate::Creations(items) => {
            profile.experiences = items;
            profile.sections.creations = true;
        }
        ProfileUpdate::Favorites(items) => {
            profile.favorites = items;
            profile.sections.favorites = true;
        }
        // A failed section keeps its skeleton: it is not "no badges", it is "not known".
        ProfileUpdate::Failed(_) | ProfileUpdate::Done => {}
    }
}

/// A profile that is still streaming in. The seed is already drawable; `profile` becomes
/// more complete as `rx` delivers sections.
impl crate::types::Game {
    /// Heap bytes this game holds: its own struct plus the strings it owns.
    pub fn tracked_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.len()
            + self.creator.len()
            + self.description.len()
            + self.players_text.len()
            + self.rating_text.len()
    }
}

impl crate::types::FriendProfile {
    /// Heap bytes this profile holds: its own struct, its strings, and the games, badges and
    /// avatar assets it carries. The RAM budget charges the profile shelf with this instead of
    /// a flat per-entry guess.
    pub fn tracked_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.user.name.len()
            + self.user.display.len()
            + self.user.headshot.len()
            + self.user.location.len()
            + self.description.len()
            + self.created.len()
            + self.avatar.len()
            + self
                .playing_game
                .as_ref()
                .map(|game| game.tracked_bytes())
                .unwrap_or(0)
            + self
                .experiences
                .iter()
                .map(|game| game.tracked_bytes())
                .sum::<usize>()
            + self
                .favorites
                .iter()
                .map(|game| game.tracked_bytes())
                .sum::<usize>()
            + std::mem::size_of::<crate::types::PlayerBadge>() * self.badges.len()
            + std::mem::size_of::<crate::types::AvatarAsset>() * self.currently_wearing.len()
    }
}

pub struct FriendStream {
    pub uid: u64,
    pub rx: Receiver<ProfileUpdate>,
    pub profile: FriendProfile,
    pub err: Option<String>,
    pub finished: bool,
}

impl FriendStream {
    pub fn new(uid: u64, seed: FriendProfile, rx: Receiver<ProfileUpdate>) -> Self {
        Self {
            uid,
            rx,
            profile: seed,
            err: None,
            finished: false,
        }
    }

    /// Takes everything that has arrived. Returns how many sections were applied and
    /// whether the loader has finished, so the caller can repaint only on change.
    pub fn drain(&mut self) -> (usize, bool) {
        if self.finished {
            return (0, true);
        }
        let mut applied = 0;
        loop {
            match self.rx.try_recv() {
                Ok(update) => match update {
                    ProfileUpdate::Done => {
                        self.finished = true;
                        return (applied, true);
                    }
                    ProfileUpdate::Failed(e) => {
                        // Keep whatever else arrived; the page shows a quiet warning.
                        if self.err.is_none() {
                            self.err = Some(e);
                        }
                    }
                    update => {
                        apply(&mut self.profile, update);
                        applied += 1;
                    }
                },
                Err(TryRecvError::Empty) => return (applied, false),
                Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    return (applied, true);
                }
            }
        }
    }
}

struct Entry {
    profile: FriendProfile,
    stored: Instant,
    /// Monotonic use counter; the smallest is the least recently used.
    used: u64,
}

/// A bounded, least-recently-used shelf of profiles with a stale-while-revalidate TTL.
pub struct ProfileCache {
    entries: HashMap<u64, Entry>,
    seq: u64,
    capacity: usize,
    ttl: Duration,
}

impl Default for ProfileCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileCache {
    pub fn new() -> Self {
        Self::with_limits(CAPACITY, TTL)
    }

    pub fn with_limits(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            seq: 0,
            capacity: capacity.max(1),
            ttl,
        }
    }

    /// The profile, marked as just used. Cloned: the caller usually wants to paint it
    /// into page state, and a profile is one user record plus a few short lists.
    pub fn get(&mut self, id: u64) -> Option<FriendProfile> {
        self.get_at(id, Instant::now())
    }

    pub fn get_at(&mut self, id: u64, now: Instant) -> Option<FriendProfile> {
        let _ = now;
        self.seq += 1;
        let seq = self.seq;
        let entry = self.entries.get_mut(&id)?;
        entry.used = seq;
        Some(entry.profile.clone())
    }

    /// True when the profile is missing or older than the TTL: paint it if present, then
    /// refresh it in the background.
    pub fn needs_refresh(&self, id: u64) -> bool {
        self.needs_refresh_at(id, Instant::now())
    }

    pub fn needs_refresh_at(&self, id: u64, now: Instant) -> bool {
        match self.entries.get(&id) {
            None => true,
            // A profile whose sections never all arrived (a failed endpoint) is useful to
            // paint but always worth asking about again.
            Some(e) => {
                !is_complete(e.profile.sections)
                    || now.saturating_duration_since(e.stored) >= self.ttl
            }
        }
    }

    /// True when the profile is cached, complete and still inside the TTL.
    pub fn is_fresh(&self, id: u64) -> bool {
        !self.needs_refresh(id)
    }

    pub fn insert(&mut self, profile: FriendProfile) {
        self.insert_at(profile, Instant::now());
    }

    /// Inserts with an explicit fetch time, so a stale-while-revalidate test does not have
    /// to wait five minutes.
    pub fn insert_at(&mut self, profile: FriendProfile, at: Instant) {
        self.seq += 1;
        let used = self.seq;
        self.entries.insert(
            profile.user.id,
            Entry {
                profile,
                stored: at,
                used,
            },
        );
        while self.entries.len() > self.capacity {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.remove(&id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Bytes the shelf holds: every cached profile's tracked heap plus its LRU entry.
    ///
    /// This is what the RAM budget charges the profile cache with, so a full shelf (32
    /// profiles with their game lists) counts against the same ceiling as the images.
    pub fn bytes(&self) -> usize {
        self.entries
            .values()
            .map(|entry| entry.profile.tracked_bytes() + std::mem::size_of::<Entry>())
            .sum()
    }

}
