
//! ## Pendulum Example:
//! 
//! ```rust
//! extern crate pendulum;
//! 
//! use std::time::Duration;
//! use std::thread;
//! 
//! use pendulum::{Pendulum, HashedWheelBuilder};
//! 
//! #[derive(Debug, PartialEq, Eq)]
//! struct SomeData(usize);
//! 
//! fn main() {
//!     // Create a pendulum with mostly default configration
//!     let mut wheel = HashedWheelBuilder::default()
//!         // Tick duration defines the resolution for our timer (all timeouts will be a multiple of this)
//!         .with_tick_duration(Duration::from_millis(100))
//!         .build();
//! 
//!     // Insert a timeout and store the token, we can use this to cancel the timeout
//!     let token = wheel.insert_timeout(Duration::from_millis(50), SomeData(5)).unwrap();
//! 
//!     // Tick our wheel after the given duration (100 ms)
//!     thread::sleep(wheel.tick_duration());
//! 
//!     // Tell the wheel that it can perform a tick
//!     wheel.tick();
//! 
//!     // Retrieve any expired timeouts
//!     while let Some(timeout) = wheel.expired_timeout() {
//!         assert_eq!(SomeData(5), timeout);
//!     }
//!     
//!     // If we tried to remove the timeout using the token, we get None (already expired)
//!     assert_eq!(None, wheel.remove_timeout(token));
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
mod wheel;

pub use pendulum::{Pendulum, Token};
pub use wheel::{HashedWheel, HashedWheelBuilder};