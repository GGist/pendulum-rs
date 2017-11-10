//! Errors that a `Pendulum` can produce.

use std::fmt;
use std::fmt::Debug;
use std::fmt::Formatter;

/// Result type for a `Pendulum`.
pub type PendulumResult<T1, T2> = Result<T1, PendulumError<T2>>;

/// Error type for `Pendulum` operations.
pub struct PendulumError<T> {
    item: T,
    kind: PendulumErrorKind
}

impl<T> PendulumError<T> {
    /// Create a new `PendulumError`.
    pub fn new(item: T, kind: PendulumErrorKind) -> PendulumError<T> {
        PendulumError{ item: item, kind: kind }
    }

    /// Retrieve the error kind of the `PendulumError`.
    pub fn kind(&self) -> &PendulumErrorKind {
        &self.kind
    }

    /// Retrieve the item contained within the error.
    pub fn item(&self) -> &T {
        &self.item
    }

    /// Break the error down into its parts.
    pub fn into_parts(self) -> (T, PendulumErrorKind) {
        (self.item, self.kind)
    }
}

impl<T> Debug for PendulumError<T> where T: Debug {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        f.write_fmt(format_args!("PendulumError {{ item: {:?}, kind: {:?} }}", self.item, self.kind))
    }
}

/// Enumeration of `Pendulum` errors.
#[derive(Debug)]
pub enum PendulumErrorKind {
    MaxCapacityReached,
    MaxTimeoutExceeded
}