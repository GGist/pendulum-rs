#[macro_use]
extern crate log;
extern crate slab;

#[cfg(feature = "future")]
extern crate crossbeam;
#[cfg(feature = "future")]
extern crate futures;

pub mod error;

#[cfg(feature = "future")]
pub mod future;

mod pendulum;

pub use pendulum::{Pendulum, PendulumBuilder, Token, PendulumTicker};