
//! ## Pendulum Example:
//! 
//! ```rust
//! extern crate pendulum;
//! 
//! use std::time::Duration;
//! use std::thread;
//! 
//! use pendulum::PendulumBuilder;
//! 
//! #[derive(Debug, PartialEq, Eq)]
//! struct SomeData(usize);
//! 
//! fn main() {
//!     // Create a pendulum with mostly default configration
//!     let mut pendulum = PendulumBuilder::default()
//!         // Tick duration defines the resolution for our timer (all timeouts will be a multiple of this)
//!         .with_tick_duration(Duration::from_millis(100))
//!         .build();
//! 
//!     // Insert a timeout and store the token, we can use this to cancel the timeout
//!     let token = pendulum.insert_timeout(Duration::from_millis(50), SomeData(5)).unwrap();
//! 
//!     // Tick our pendulum after the given duration (100 ms)
//!     thread::sleep(pendulum.ticker().tick_duration());
//! 
//!     // Tell the pendulum that it can perform a tick
//!     pendulum.ticker().tick();
//! 
//!     // Retrieve any expired timeouts
//!     while let Some(timeout) = pendulum.expired_timeout() {
//!         assert_eq!(SomeData(5), timeout);
//!     }
//!     
//!     // If we tried to remove the timeout using the token, we get None (already expired)
//!     assert_eq!(None, pendulum.remove_timeout(token));
//! }
//! ```

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