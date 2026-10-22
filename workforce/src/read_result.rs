//! A workforce read value and the freshness a caller may rely on (Bilimbi
//! `Bilimbi.People.Workforce.ReadResult`).
//!
//! Native reads are current. An adapter that serves a cached copy of another
//! system's workforce answers `stale` with the time it last confirmed the
//! value, or `unavailable` with a reason. The People connector is the first
//! consumer that answers anything but current.
use jiff::Timestamp;

/// How far a caller may trust the value beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Read from the native source just now.
    Current,
    /// A cached copy, last confirmed at this instant.
    Stale(Timestamp),
    /// No value could be read; the reason is for a notice, never for logic.
    Unavailable(String),
}

/// A value with its freshness. `value` is `None` only when unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadResult<T> {
    pub value: Option<T>,
    pub freshness: Freshness,
}

/// `require_current` on a stale or unavailable result (Bilimbi
/// `{:error, {:not_current, freshness}}`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The workforce read is not current")]
pub struct NotCurrent(pub Freshness);

impl<T> ReadResult<T> {
    pub fn current(value: T) -> Self {
        Self {
            value: Some(value),
            freshness: Freshness::Current,
        }
    }
    pub fn stale(value: T, last_confirmed_at: Timestamp) -> Self {
        Self {
            value: Some(value),
            freshness: Freshness::Stale(last_confirmed_at),
        }
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            freshness: Freshness::Unavailable(reason.into()),
        }
    }
    pub fn is_current(&self) -> bool {
        self.freshness == Freshness::Current
    }
    /// The value only when it is current; a stale or unavailable result is
    /// the refusal a caller that must not act on old facts reports.
    pub fn require_current(self) -> Result<T, NotCurrent> {
        match (self.freshness, self.value) {
            (Freshness::Current, Some(value)) => Ok(value),
            (Freshness::Current, None) => Err(NotCurrent(Freshness::Unavailable("empty".into()))),
            (freshness, _) => Err(NotCurrent(freshness)),
        }
    }
    /// The same freshness around a derived value.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ReadResult<U> {
        ReadResult {
            value: self.value.map(f),
            freshness: self.freshness,
        }
    }
}
