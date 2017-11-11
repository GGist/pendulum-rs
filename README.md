# Pendulum
[![Build Status](https://travis-ci.org/GGist/pendulum-rs.svg?branch=master)](https://travis-ci.org/GGist/pendulum-rs) [![Build status](https://ci.appveyor.com/api/projects/status/y9udu8r9r4lae291/branch/master?svg=true)](https://ci.appveyor.com/project/GGist/pendulum-rs/branch/master) [![Documentation](https://docs.rs/pendulum/badge.svg)](https://docs.rs/pendulum) [![Crates.io](https://img.shields.io/crates/v/pendulum.svg)](https://crates.io/crates/pendulum)

Data structures and runtimes for efficient timer management.

## Usage

```Cargo.toml```:
```
[dependencies]
pendulum = "0.2"
```

```lib.rs/main.rs```:
```
extern crate pendulum;
```

## Examples

Usage of the futures base `Timer` runtime:
```rust
extern crate pendulum;
extern crate futures;

use std::time::Duration;

use futures::Stream;
use futures::sync::mpsc;

use pendulum::future::{TimerBuilder, TimedOut};

#[derive(Debug, PartialEq, Eq)]
enum PeerMessage {
    KeepAlive,
    DoSomething
}

impl From<TimedOut> for PeerMessage {
    fn from(_: TimedOut) -> PeerMessage {
        PeerMessage::KeepAlive
    }
}

fn main() {
    // Create a timer with the default configuration
    let timer = TimerBuilder::default()
        .build();

    // Assume some other part of the application sends messages to some peer
    let (send, recv) = mpsc::unbounded();

    // Application sent the peer a single message
    send.unbounded_send(PeerMessage::DoSomething)
        .unwrap();

    // Wrap the receiver portion (a `Stream`), in a `Heartbeat` stream
    let mut heartbeat = timer.heartbeat(Duration::from_millis(100), recv)
        .unwrap()
        .wait();

    // Should receive the applications message
    assert_eq!(PeerMessage::DoSomething, heartbeat.next().unwrap().unwrap());

    // Application only sent one message, timer will continuously send keep alives
    // if 100 ms goes by without the original receiver receiving any messages
    assert_eq!(PeerMessage::KeepAlive, heartbeat.next().unwrap().unwrap());
    assert_eq!(PeerMessage::KeepAlive, heartbeat.next().unwrap().unwrap());
}
```

Usage of the `Pendulum` data structure:
```rust
extern crate pendulum;

use std::time::Duration;
use std::thread;

use pendulum::PendulumBuilder;

#[derive(Debug, PartialEq, Eq)]
struct SomeData(usize);

fn main() {
    // Create a pendulum with mostly default configration
    let mut pendulum = PendulumBuilder::default()
        // Tick duration defines the resolution for our timer (all timeouts will be a multiple of this)
        .with_tick_duration(Duration::from_millis(100))
        .build();

    // Insert a timeout and store the token, we can use this to cancel the timeout
    let token = pendulum.insert_timeout(Duration::from_millis(50), SomeData(5)).unwrap();

    // Tick our pendulum after the given duration (100 ms)
    thread::sleep(pendulum.ticker().tick_duration());

    // Tell the pendulum that it can perform a tick
    pendulum.ticker().tick();

    // Retrieve any expired timeouts
    while let Some(timeout) = pendulum.expired_timeout() {
        assert_eq!(SomeData(5), timeout);
    }
    
    // If we tried to remove the timeout using the token, we get None (already expired)
    assert_eq!(None, pendulum.remove_timeout(token));
}
```

## References

* tokio-timer: https://github.com/tokio-rs/tokio-timer

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
