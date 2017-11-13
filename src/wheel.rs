use pendulum::Pendulum;
use pendulum::{self, Token};
use std::time::Duration;
use std::cmp;
use std::usize;
use slab::Slab;
use error::{PendulumError, PendulumErrorKind, PendulumResult};

const DEFAULT_TICK_MILLIS:        u64   = 100;
const DEFAULT_NUM_SLOTS:          usize = 2048;
const DEFAULT_INIT_CAPACITY:      usize = 2048;
const DEFAULT_MAX_CAPACITY:       usize = 16384;

/// Builder for configuring different options for a `HashedWheel`.
pub struct HashedWheelBuilder {
    tick_duration: Duration,
    num_slots: usize,
    init_capacity: usize,
    max_capacity: usize,
    max_timeout: Option<Duration>
}

impl HashedWheelBuilder {
    /// Sets the tick duration which sets the resolution for the timer.
    /// 
    /// Any timeouts will be rounded up based on the tick duration.
    pub fn with_tick_duration(mut self, duration: Duration) -> HashedWheelBuilder {
        self.tick_duration = duration;
        self
    }

    /// Sets the number of slots used for ticks.
    /// 
    /// Controls how many slots we have for timeouts. The less number of collisions
    /// there are (where a collision is when two or more items in the same slot have
    /// different timeouts), the more efficient operations we can perform.
    pub fn with_num_slots(mut self, num_slots: usize) -> HashedWheelBuilder {
        self.num_slots = num_slots;
        self
    }

    /// Sets the initial capacity for timer storage.
    /// 
    /// This is the number of timeouts we can initially store.
    pub fn with_init_capacity(mut self, init_capacity: usize) -> HashedWheelBuilder {
        self.init_capacity = init_capacity;
        self
    }

    /// Sets the maximum capacity for timer storage.
    /// 
    /// This is the maximum number of timeouts we can ever store.
    pub fn with_max_capacity(mut self, max_capacity: usize) -> HashedWheelBuilder {
        self.max_capacity = max_capacity;
        self
    }

    /// Sets the maximum timeout for any timer.
    /// 
    /// Defaults to `tick_duration * num_slots`.
    pub fn with_max_timeout(mut self, timeout: Duration) -> HashedWheelBuilder {
        self.max_timeout = Some(timeout);
        self
    }

    /// Get the tick duration that was set.
    pub fn tick_duration(&self) -> Duration {
        self.tick_duration
    }

    /// Get the number of slots that were set.
    pub fn num_slots(&self) -> usize {
        self.num_slots
    }

    /// Get the initial capacity for timer storage that was set.
    pub fn init_capacity(&self) -> usize {
        // Doesnt make sense to allocate more than the max
        cmp::min(self.init_capacity, self.max_capacity)
    }

    /// Get the maximum capacity for timer storage that was set.
    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    /// Get the maximum timeout that was set.
    pub fn max_timeout(&self) -> Duration {
        self.max_timeout.unwrap_or_else(|| {
            self.tick_duration * self.num_slots as u32
        })
    }

    /// Build a new `HashedWheel` from the current builder.
    pub fn build<T>(self) -> HashedWheel<T> {
        self.into()
    }
}

impl Default for HashedWheelBuilder {
    fn default() -> HashedWheelBuilder {
        HashedWheelBuilder{ tick_duration: Duration::from_millis(DEFAULT_TICK_MILLIS), num_slots: DEFAULT_NUM_SLOTS,
            init_capacity: DEFAULT_INIT_CAPACITY, max_capacity: DEFAULT_MAX_CAPACITY, max_timeout: None }
    }
}

//--------------------------------------------------------------//

struct Timeout<T> {
    item:       T,
    slot_index: usize,
    abs_expire: u64,
    prev:       Option<Token>,
    next:       Option<Token>
}

/// `HashedWheel` representing a timer that stores timeouts for type `T`.
pub struct HashedWheel<T> {
    slots: Vec<Option<Token>>,
    storage: Slab<Timeout<T>>,
    // Current free should always stay BEHIND current tick
    curr_free: u64,
    // Timeouts in the current tick slot have NOT expired yet
    curr_tick: u64,
    max_capacity: usize,
    max_timeout: Duration,
    tick_duration: Duration
}

impl<T> From<HashedWheelBuilder> for HashedWheel<T> {
    fn from(builder: HashedWheelBuilder) -> HashedWheel<T> {
        let slots = vec![Option::None; builder.num_slots()];
        let storage = Slab::with_capacity(builder.init_capacity());

        HashedWheel{ slots: slots, storage: storage, curr_free: 0, curr_tick: 0,
            max_capacity: builder.max_capacity(), max_timeout: builder.max_timeout(),
            tick_duration: builder.tick_duration() }
    }
}

impl<T> Pendulum<T> for HashedWheel<T> {
    fn insert_timeout(&mut self, timeout: Duration, item: T) -> PendulumResult<Token, T> {
        if self.storage.len() == self.max_capacity() {
            Err(PendulumError::new(item, PendulumErrorKind::MaxCapacityReached))
        } else if timeout > self.max_timeout {
            Err(PendulumError::new(item, PendulumErrorKind::MaxTimeoutExceeded))
        } else {
            // Figure out the absolute number of ticks that this timeout needs
            let abs_ticks = abs_tick_from_duration(self.curr_tick, self.tick_duration(), timeout);
            // Figure out what slot we place this in based on its ticks required
            let slot_index = (abs_ticks % self.slots.len() as u64) as usize;

            // While we keep getting some next insert token, keep advancing
            let mut opt_curr_token = self.slots[slot_index];
            while let Some(next_token) = self.next_insert_token(opt_curr_token, abs_ticks) {
                opt_curr_token = Some(next_token);
            }

            // Check where our insertion point ended up at
            match opt_curr_token {
                Some(curr_token) => {
                    let inner_curr_token = pendulum::retrieve_token(curr_token);

                    // We ended up at a real token, could either insert before or after
                    let (pivot_abs_expire, pivot_prev, pivot_next) = {
                        let pivot = &self.storage[inner_curr_token];

                        (pivot.abs_expire, pivot.prev, pivot.next)
                    };

                    // Check if we need to insert before or after the pivot
                    if pivot_abs_expire <= abs_ticks {
                        // Insert after pivot

                        // Insert our token, linking in the previous and next pivot
                        let raw_token = self.storage.insert(Timeout{ item: item, slot_index: slot_index,
                            abs_expire: abs_ticks, prev: Some(curr_token), next: pivot_next });
                        let token = pendulum::create_token(raw_token);
                        // Update the pivot to point next to our new timeout
                        self.storage.get_mut(inner_curr_token).unwrap().next = Some(token);
                    } else {
                        // Insert before pivot

                        // Insert our token, linking in the previous and next pivot
                        let raw_token = self.storage.insert(Timeout{ item: item, slot_index: slot_index,
                            abs_expire: abs_ticks, prev: pivot_prev, next: Some(curr_token) });
                        let token = pendulum::create_token(raw_token);
                        // Update the pivot to point prev to our new timeout
                        self.storage.get_mut(inner_curr_token).unwrap().prev = Some(token);

                        // Check if the pivots previous was None, if so, update the slot Token to point to the new head
                        if pivot_prev.is_none() {
                            self.slots[slot_index] = Some(token);
                        }
                    }

                    Ok(curr_token)
                },
                None => {
                    // Current token is None, insert at the head of the list
                    let raw_token = self.storage.insert(Timeout{ item: item, slot_index: slot_index,
                        abs_expire: abs_ticks, prev: None, next: None });
                    let token = pendulum::create_token(raw_token);

                    self.slots[slot_index] = Some(token);
                    Ok(token)
                }
            }
        }
    }

    fn remove_timeout(&mut self, token: Token) -> Option<T> {
        let inner_token = pendulum::retrieve_token(token);

        // Find the token in our slab, remove it
        let opt_timeout = if self.storage.contains(inner_token) {
            Some(self.storage.remove(inner_token))
        } else {
            None
        };

        opt_timeout.map(|timeout| {
            // If the entry was first (prev is None), need to update slots to point to next and update next prev to None
            // Else if the entry has prev Some and next Some, need to update prev next and next prev
            // Else if the entry has prev Some and next None, need to update prev next to None
            match (timeout.prev, timeout.next) {
                (Some(prev), Some(next)) => {
                    // In the middle of a list, update prev and next
                    self.storage.get_mut(pendulum::retrieve_token(prev)).unwrap().next = Some(next);
                    self.storage.get_mut(pendulum::retrieve_token(next)).unwrap().prev = Some(prev);
                },
                (None, Some(next)) => {
                    // At the front of a list with another element, update slots and next
                    self.storage.get_mut(pendulum::retrieve_token(next)).unwrap().prev = None;
                    self.slots[timeout.slot_index] = Some(next);
                },
                (Some(prev), None) => {
                    // At the end of a list, update next
                    self.storage.get_mut(pendulum::retrieve_token(prev)).unwrap().next = None;  
                },
                (None, None) => {
                    // At the front of a list with no other element, udpate slots
                    self.slots[timeout.slot_index] = None;
                }
            }

            timeout.item
        })
    }

    fn expired_timeout(&mut self) -> Option<T> {
        // While current free is less than current tick
        while self.curr_free < self.curr_tick {
            let curr_slot = self.current_free_slot();

            // Go to the head entry of the list, check if the expire is less than or equal to the current expire
            let opt_remove_token = self.slots[curr_slot].and_then(|token| {
                let timeout = &self.storage[pendulum::retrieve_token(token)];

                if timeout.abs_expire <= self.curr_free {
                    Some(token)
                } else {
                    None
                }
            });

            // If we got a token corresponding to a timeout that we can remove, them remove it and return it now
            match opt_remove_token {
                Some(remove_token) => { return self.remove_timeout(remove_token) },
                None               => ()
            }

            // Otherwise, the head of the (sorted) list wasnt timed out, continue on
            self.curr_free += 1;
        }
        
        // We caught up to our tick pointer, no more free slots to check
        None
    }

    fn tick(&mut self) {
        self.curr_tick += 1;
    }

    fn tick_duration(&self) -> Duration {
        self.tick_duration
    }

    fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    fn max_timeout(&self) -> Duration {
        self.max_timeout
    }
}

impl<T> HashedWheel<T> {
    /// Continues to returun Some if the next token in the list is more suitable for insertion. Should only insert when the passed
    /// token causes this function to return None, as that signifies the passed token references a timeout that should be used to
    /// insert the timeout with entry_abs_ticks.
    fn next_insert_token(&self, opt_token: Option<Token>, entry_abs_ticks: u64) -> Option<Token> {
        opt_token.and_then(|token| {
            let timeout = &self.storage[pendulum::retrieve_token(token)];

            // If timeout expire is greater than or equal to new entry ticks, OR next is None, we insert at the current token
            if timeout.abs_expire >= entry_abs_ticks || timeout.next.is_none() {
                None
            } else {
                // Otherwise, return the next entry that the caller can check
                timeout.next
            }
        })
    }

    /// Retrive the current free slot, based on the current free pointer.
    fn current_free_slot(&self) -> usize {
        (self.curr_free % self.slots.len() as u64) as usize
    }
}

fn abs_tick_from_duration(curr_tick: u64, tick_duration: Duration, timeout_duration: Duration) -> u64 {
    // Checked division ensures that if any of our tick components is zero, the divide will leave us with None
    let opt_seconds_div = timeout_duration.as_secs()
        .checked_div(tick_duration.as_secs())
        .or_else(|| {
            // If number of tick seconds is zero, convert timeout to nanoseconds, and get ticks from that
            (timeout_duration.as_secs() * 1_000_000_000).checked_div(tick_duration.subsec_nanos() as u64)
        });
    let opt_nanos_div = timeout_duration.subsec_nanos().checked_div(tick_duration.subsec_nanos());

    // If we tried to divie by zero, even at the nanosecond level, then our tick duration was most likely zero,
    // in that case, we effectively return the current tick as the timeout, as a tick duration of zero probably
    // means that all timeouts now and in the future, are all expired, simultaneously...at the same time :)
    curr_tick + opt_seconds_div.unwrap_or(0) + opt_nanos_div.unwrap_or(0) as u64
}

#[cfg(test)]
mod tests {
    use super::HashedWheelBuilder;
    use pendulum::Pendulum;
    use error::{PendulumError, PendulumErrorKind};

    use std::time::Duration;

    #[test]
    fn positive_remove_timeout() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::from_millis(100))
            .build();

        let token = wheel.insert_timeout(Duration::from_millis(1), ()).unwrap();
        assert_eq!(Some(()), wheel.remove_timeout(token));

        wheel.tick();
        assert_eq!(None, wheel.expired_timeout());
    }

    #[test]
    fn positive_remove_timeout_after_expired() {
        let mut wheel = HashedWheelBuilder::default()
            .build();

        let token = wheel.insert_timeout(Duration::from_millis(1), ()).unwrap();
        wheel.tick();

        assert_eq!(Some(()), wheel.remove_timeout(token));
        assert_eq!(None, wheel.expired_timeout());
    }

    #[test]
    fn positive_timeout_equal_to_tick_rounded_up() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::from_millis(100))
            .with_num_slots(100)
            .build();

        wheel.insert_timeout(Duration::from_millis(100), ()).unwrap();

        wheel.tick();
        assert_eq!(None, wheel.expired_timeout());

        wheel.tick();
        assert_eq!(Some(()), wheel.expired_timeout());
    }

    #[test]
    fn positive_expired_timeout_is_removed() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::from_millis(100))
            .with_num_slots(100)
            .build();

        wheel.insert_timeout(Duration::from_millis(150), ()).unwrap();
        wheel.tick();
        wheel.tick();

        assert_eq!(Some(()), wheel.expired_timeout());

        // Go around all the slots, make sure it was really removed...
        for _ in 0..100 {
            assert_eq!(None, wheel.expired_timeout());
            wheel.tick();
        }
        assert_eq!(None, wheel.expired_timeout());
    }


    #[test]
    fn positive_timeout_less_than_tick_rounded_up() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::from_millis(100))
            .build();

        wheel.insert_timeout(Duration::from_millis(50), ()).unwrap();
        assert_eq!(None, wheel.expired_timeout());

        wheel.tick();
        assert_eq!(Some(()), wheel.expired_timeout());
    }

    #[test]
    fn positive_timeout_greater_than_tick_rounded_up() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::from_millis(100))
            .build();

        wheel.insert_timeout(Duration::from_millis(150), ()).unwrap();
        assert_eq!(None, wheel.expired_timeout());

        wheel.tick();
        assert_eq!(None, wheel.expired_timeout());

        wheel.tick();
        assert_eq!(Some(()), wheel.expired_timeout());
        assert_eq!(None, wheel.expired_timeout());
    }

    #[test]
    fn positive_tick_duration_zero_times_out_everything() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::from_millis(0))
            .with_max_timeout(Duration::from_millis(20000))
            .build();

        wheel.insert_timeout(Duration::from_millis(150), ()).unwrap();
        assert_eq!(None, wheel.expired_timeout());
        wheel.tick();
        assert_eq!(Some(()), wheel.expired_timeout());

        wheel.insert_timeout(Duration::from_millis(20000), ()).unwrap();
        assert_eq!(None, wheel.expired_timeout());
        wheel.tick();
        assert_eq!(Some(()), wheel.expired_timeout());
    }

    #[test]
    fn positive_nano_tick_duration_milli_timeout() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::new(0, 1000))
            .with_max_timeout(Duration::from_millis(1))
            .build();

        wheel.insert_timeout(Duration::from_millis(1), ()).unwrap();

        for _ in 0..1_001 {
            assert_eq!(None, wheel.expired_timeout());
            wheel.tick();
        }
        assert_eq!(Some(()), wheel.expired_timeout());
    }

    #[test]
    fn positive_nano_tick_duration_nano_timeout() {
        let mut wheel = HashedWheelBuilder::default()
            .with_tick_duration(Duration::new(0, 1000))
            .build();

        wheel.insert_timeout(Duration::new(0, 2000), ()).unwrap();

        for _ in 0..3 {
            assert_eq!(None, wheel.expired_timeout());
            wheel.tick();
        }
        assert_eq!(Some(()), wheel.expired_timeout());
    }

    #[test]
    fn negative_max_timeout_exceeded() {
        let mut wheel = HashedWheelBuilder::default()
            .with_max_timeout(Duration::from_millis(100))
            .build();

        let result = wheel.insert_timeout(Duration::from_millis(101), ());
        match result.as_ref().map_err(PendulumError::kind) {
            Err(&PendulumErrorKind::MaxTimeoutExceeded) => (),
            _                                              => panic!("MaxTimeoutExceeded Not Returned")
        }
    }

    #[test]
    fn negative_max_timers_exceeded() {
        let mut wheel = HashedWheelBuilder::default()
            .with_max_capacity(3)
            .build();

        wheel.insert_timeout(Duration::from_millis(0), ()).unwrap();
        wheel.insert_timeout(Duration::from_millis(0), ()).unwrap();
        wheel.insert_timeout(Duration::from_millis(0), ()).unwrap();

        let result = wheel.insert_timeout(Duration::from_millis(0), ());
        match result.as_ref().map_err(PendulumError::kind) {
            Err(&PendulumErrorKind::MaxCapacityReached) => (),
            _                                              => panic!("MaxCapacityReached Not Returned")
        }
    }
}