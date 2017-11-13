
//! Data structures and runtimes for timer management.
//! 
//! This library provides various data structures for managing coarse grain timers.
//! 
//! #### Data Structures
//! 
//! **Overview**
//! 
//! We provide various data structures for timer management, each with their own tradeoffs and suited for different application.
//! 
//! * For applications that set shorted lived timeouts, all around the same duration, consider using a `HashedWheel`, as it will
//! require minimal bookkeeping at runtime for operations such as ticking, or removing expired timers.
//!     * Network communication
//!     * Cache for short lived items
//! 
//! * For applications that set long lived timeouts, or timeouts spread across many different durations, considuer using a `HierarchicalWheel`, as
//! it will accept a variety of different durations, without loss of precision.
//!     * Cache for long lived items
//!     * Varied timeouts from user input
//! 
//! **Hashed Wheel**
//! 
//! Stores all timers in a fixed number of slots (`num_slots`), where each slot represents some fixed period of time (`tick_duration`).
//! The slots we maintain are used as a circular buffer, where after each tick, we advance our position in the buffer. Each tick occurrs
//! after approximately `tick_duration` time has passed.
//! 
//! **Note**: One slot could have multiple timers of differing durations, these timers will be sorted based on tick expiration. However, if
//! the wheel is configured such that one slot contains timers expiring at different ticks, insertion time will fall back to O(n) instead of O(1).
//! By default, `max_timeout` is set equal to `tick_duration` * `num_slots` - `1 ns`, to ensure that all timers inserted will be O(1) inserts, though
//! you can change this if required.
//! 
//! **Hierarchical Wheel**
//! 
//! *Not Implemented Yet*
//! 
//! #### Runtimes
//! 
//! **Overview**
//! 
//! We provide various runtimes on top of the base data structures for timer management, each being generic over the type of `Pendulum` that you choose.
//! 
//! Runtimes are useful because they allow you to not care about how the underlying data structure is getting ticked, or when we should check for an expired timer.
//! 
//! **Futures**
//! 
//! Runs the actual `Pendulum` in a separate thread that handles accepting requests for new timers, as well as making sure the `Pendulum` is ticked correctly, and
//! finally notifications for expired timeouts.
//! 
//! Since the futures library is based around the concept of readiness, via `Task`, we can have callers doing other useful work and make it so that callers aren't
//! continuously checking if a timer is up, only when it is actually up with the background timer thread signal readiness, so that the `Future`s library will
//! poll the timer again.
//! 
//! **Note**: Timer bounds (on both the duration and capacity), is checked before returning a `Future`/`Stream` from the `Timer` object, so the only `Error` being
//! returned from either of those objects is related to timer expiration, which makes it easy to integrate the error handling into your application correctly.
//! 
//! ## Hashed Wheel Example:
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