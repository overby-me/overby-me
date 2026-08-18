//! Diagnostics.
//!
//! On the drone these went out of the USB serial port; here they go into a
//! bounded ring the caller can read, which is the same thing minus the wire.
//!
//! Two things about the C module did not survive the port. It appended its
//! `"W: "` / `"E: "` markers to the end of the message rather than the front,
//! which is a plain mistake, so an entry carries its severity and sender as
//! fields instead and the consumer decides how to write them. And its mock
//! build printed every message regardless of level, which defeats the point of
//! having a level; the port always filters.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// How much gets through.
///
/// Ordered as the C enum is ordered, so the comparisons below read the same:
/// `LOG_NONE = -1` through `LOG_ALL = 2`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Level {
    /// Nothing at all, not even errors.
    #[default]
    None,
    /// Errors and serious warnings.
    OnlyErrors,
    /// Also ordinary warnings.
    Debug,
    /// Also messages.
    All,
}

/// The component a message came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sender {
    Sonar,
    Ir,
    Io,
    Laser,
    Fc,
    Map,
    Board,
}

/// How bad it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// Ordinary running commentary.
    Message,
    /// Something is off but the drone can carry on.
    Warning,
    /// Something is off and the ignore list does not get to hide it.
    SeriousWarning,
    /// The drone should land. Never filtered.
    Error,
}

/// One recorded diagnostic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub severity: Severity,
    pub sender: Sender,
    pub message: &'static str,
}

/// How many entries are kept before the oldest is dropped.
const CAPACITY: usize = 128;

/// The firmware's log.
#[derive(Clone, Debug)]
pub struct Log {
    level: Level,
    disabled: Vec<Sender>,
    entries: VecDeque<Entry>,
    dropped: usize,
}

impl Log {
    /// A log at the given level, as `init_logging`.
    #[must_use]
    pub fn new(level: Level) -> Self {
        Self {
            level,
            disabled: Vec::new(),
            entries: VecDeque::new(),
            dropped: 0,
        }
    }

    /// The level currently in force.
    #[must_use]
    pub fn level(&self) -> Level {
        self.level
    }

    /// Changes the level.
    pub fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    /// Stops listening to a component. Repeating a device is a no-op, as it is
    /// in the C list.
    pub fn disable_device(&mut self, device: Sender) {
        if !self.disabled.contains(&device) {
            self.disabled.push(device);
        }
    }

    /// Whether a component has been disabled.
    #[must_use]
    pub fn is_ignored(&self, sender: Sender) -> bool {
        self.disabled.contains(&sender)
    }

    /// How many components are disabled.
    #[must_use]
    pub fn disabled_count(&self) -> usize {
        self.disabled.len()
    }

    /// Listens to everything again.
    pub fn enable_all_devices(&mut self) {
        self.disabled.clear();
    }

    /// Running commentary. Needs [`Level::All`].
    pub fn message(&mut self, sender: Sender, message: &'static str) {
        if !self.is_ignored(sender) && self.level == Level::All {
            self.push(Severity::Message, sender, message);
        }
    }

    /// A warning. Needs more than [`Level::OnlyErrors`].
    pub fn warning(&mut self, sender: Sender, message: &'static str) {
        if !self.is_ignored(sender) && self.level > Level::OnlyErrors {
            self.push(Severity::Warning, sender, message);
        }
    }

    /// A warning that the level still gates but that matters more. Needs at
    /// least [`Level::OnlyErrors`].
    pub fn serious_warning(&mut self, sender: Sender, message: &'static str) {
        if !self.is_ignored(sender) && self.level >= Level::OnlyErrors {
            self.push(Severity::SeriousWarning, sender, message);
        }
    }

    /// A fault the drone should land on. Recorded whatever the level, and
    /// whatever the ignore list says, because the C bypasses both.
    pub fn error(&mut self, sender: Sender, message: &'static str) {
        self.push(Severity::Error, sender, message);
    }

    /// The entries held, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    /// How many entries fell off the back of the ring.
    #[must_use]
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Throws away everything recorded so far.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
    }

    fn push(&mut self, severity: Severity, sender: Sender, message: &'static str) {
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
            self.dropped += 1;
        }

        self.entries.push_back(Entry {
            severity,
            sender,
            message,
        });
    }
}

impl Default for Log {
    fn default() -> Self {
        Self::new(Level::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_gates_each_severity() {
        let mut log = Log::new(Level::None);
        log.message(Sender::Board, "m");
        log.warning(Sender::Board, "w");
        log.serious_warning(Sender::Board, "s");
        assert_eq!(log.entries().count(), 0);

        log.set_level(Level::OnlyErrors);
        log.message(Sender::Board, "m");
        log.warning(Sender::Board, "w");
        log.serious_warning(Sender::Board, "s");
        assert_eq!(log.entries().count(), 1);

        log.set_level(Level::Debug);
        log.message(Sender::Board, "m");
        log.warning(Sender::Board, "w");
        assert_eq!(log.entries().count(), 2);

        log.set_level(Level::All);
        log.message(Sender::Board, "m");
        assert_eq!(log.entries().count(), 3);
    }

    #[test]
    fn an_error_ignores_both_the_level_and_the_ignore_list() {
        let mut log = Log::new(Level::None);
        log.disable_device(Sender::Map);
        log.error(Sender::Map, "out of bounds");
        assert_eq!(log.entries().count(), 1);
    }

    #[test]
    fn a_disabled_device_is_only_listed_once() {
        let mut log = Log::new(Level::All);
        log.disable_device(Sender::Sonar);
        log.disable_device(Sender::Sonar);
        log.disable_device(Sender::Ir);
        assert_eq!(log.disabled_count(), 2);
        assert!(log.is_ignored(Sender::Sonar));
        assert!(!log.is_ignored(Sender::Fc));

        log.message(Sender::Sonar, "ping");
        assert_eq!(log.entries().count(), 0);
    }

    #[test]
    fn the_ring_drops_the_oldest() {
        let mut log = Log::new(Level::All);
        for _ in 0..CAPACITY + 5 {
            log.message(Sender::Board, "tick");
        }
        assert_eq!(log.entries().count(), CAPACITY);
        assert_eq!(log.dropped(), 5);
    }
}
