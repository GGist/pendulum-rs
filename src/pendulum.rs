use error::PendulumResult;
use std::time::Duration;

/// Identifier for objects inserted into a `Pendulum`.
#[derive(Copy, Clone)]
pub struct Token {
    token: usize
}

pub fn create_token(token: usize) -> Token {
    Token{ token: token }
}

pub fn retrieve_token(token: Token) -> usize {
    token.token
}

//--------------------------------------------------------------//

pub trait PendulumBuilder<P> {
    fn build<T>() -> P
        where P: ;
}

//--------------------------------------------------------------//

/// Trait for working with generic timer wheel implementations.
pub trait Pendulum<T> {
    /// Insert a timeout with the given duration and the given item into the `Pendulum`.
    fn insert_timeout(&mut self, timeout: Duration, item: T) -> PendulumResult<Token, T>;

    /// Removes the timeout corresponding with the given `Token`, if one exists.
    fn remove_timeout(&mut self, token: Token) -> Option<T>;

    /// Retrieve the next expired timeout from the `Pendulum`.
    ///
    /// This is a non-blocking operation.
    fn expired_timeout(&mut self) -> Option<T>;

    /// Tick the `Pendulum` once.
    fn tick(&mut self);

    /// Configured tick duration for this `Pendulum`.
    fn tick_duration(&self) -> Duration;

    /// Configured max timeout capacity for this `Pendulum`.
    fn max_capacity(&self) -> usize;

    /// Configured maximum timeout for this `Pendulum`.
    fn max_timeout(&self) -> Duration;
}