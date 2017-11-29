//! Futures based runtime for a `Pendulum`.

//! ## Heartbeat Example:
//! 
//! ```rust
//! extern crate pendulum;
//! extern crate futures;
//! 
//! use std::time::Duration;
//! 
//! use futures::Stream;
//! use futures::sync::mpsc;
//! 
//! use pendulum::HashedWheelBuilder;
//! use pendulum::future::{TimerBuilder, TimedOut};
//! 
//! #[derive(Debug, PartialEq, Eq)]
//! enum PeerMessage {
//!     KeepAlive,
//!     DoSomething
//! }
//! 
//! impl From<TimedOut> for PeerMessage {
//!     fn from(_: TimedOut) -> PeerMessage {
//!         PeerMessage::KeepAlive
//!     }
//! }
//! 
//! fn main() {
//!     // Create a timer with the default configuration
//!     let timer = TimerBuilder::default()
//!         .build(HashedWheelBuilder::default().build());
//! 
//!     // Assume some other part of the application sends messages to some peer
//!     let (send, recv) = mpsc::unbounded();
//! 
//!     // Application sent the peer a single message
//!     send.unbounded_send(PeerMessage::DoSomething)
//!         .unwrap();
//! 
//!     // Wrap the receiver portion (a `Stream`), in a `Heartbeat` stream
//!     let mut heartbeat = timer.heartbeat(Duration::from_millis(100), recv)
//!         .unwrap()
//!         .wait();
//! 
//!     // Should receive the applications message
//!     assert_eq!(PeerMessage::DoSomething, heartbeat.next().unwrap().unwrap());
//! 
//!     // Application only sent one message, timer will continuously send keep alives
//!     // if 100 ms goes by without the original receiver receiving any messages
//!     assert_eq!(PeerMessage::KeepAlive, heartbeat.next().unwrap().unwrap());
//!     assert_eq!(PeerMessage::KeepAlive, heartbeat.next().unwrap().unwrap());
//! }
//! ```

use pendulum::Pendulum;
use futures::Stream;
use std::sync::atomic::{Ordering, AtomicUsize};
use pendulum::Token;
use error::{PendulumResult, PendulumErrorKind, PendulumError};
use std::collections::HashMap;
use std::thread::Thread;
use std::sync::Arc;
use std::thread;
use futures::Poll;
use futures::Future;
use futures::Async;
use futures::task::{self, Task};
use std::time::Instant;
use std::time::Duration;
use crossbeam::sync::SegQueue;

const DEFAULT_CHANNEL_CAPACITY: usize = 128;

/// Builder for configuring and constructing instances of `Timer`.
pub struct TimerBuilder {
    channel_capacity: usize
}

impl TimerBuilder {
    /// Sets the channel capacity, used for communicating with the backing thread.
    pub fn with_channel_capacity(mut self, capacity: usize) -> TimerBuilder {
        self.channel_capacity = capacity;
        self
    }

    /// Capacity of the communication channel with the backing thread.
    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity
    }

    /// Construct a `Timer` with the current configuration.
    pub fn build<P>(self, pendulum: P) -> Timer
        where P: Pendulum<TimerItem> + Send + 'static {
        Timer::new(self, pendulum)
    }
}

impl Default for TimerBuilder {
    fn default() -> TimerBuilder {
        TimerBuilder{ channel_capacity: DEFAULT_CHANNEL_CAPACITY }
    }
}

//--------------------------------------------------------------//

/// Indicates a timeout has occurred.
pub struct TimedOut;

/// Convenience enum for mapping an `Item`/`Error` to something implementing `From<TimedOut>`.
#[derive(Debug, PartialEq, Eq)]
pub enum TimeoutStatus<T> {
    /// Original item/error was received.
    Original(T),
    /// Timeout item/error was received.
    TimedOut
}

impl<T> From<TimedOut> for TimeoutStatus<T> {
    fn from(_: TimedOut) -> TimeoutStatus<T> {
        TimeoutStatus::TimedOut
    }
}

//--------------------------------------------------------------//

/// `Timeout` future that will ensure an item or error is yielded after at least `Duration`.
pub struct Timeout<F> {
    opt_sleep: Option<Sleep>,
    future: F
}

impl<F> Future for Timeout<F> where F: Future, F::Error: From<TimedOut> {
    type Item = F::Item;
    type Error = F::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let opt_poll_result = self.opt_sleep.as_mut().map(Future::poll);

        match opt_poll_result {
            Some(Ok(Async::Ready(()))) => {
                // Timeout is up, drop the sleep object early
                self.opt_sleep.take();

                Err(TimedOut.into())
            },
            Some(_) => {
                // Timeout is not up, poll the future
                self.future.poll()
            },
            None => {
                // Timeout was already up
                Err(TimedOut.into())
            }
        }
    }
}

//--------------------------------------------------------------//

/// `TimeoutStream` that will ensure an item or error is yielded after at least `Duration`.
pub struct TimeoutStream<S> {
    sleep: Sleep,
    stream: S
}

impl<S> Stream for TimeoutStream<S> where S: Stream, S::Error: From<TimedOut> {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // Check if our sleep is up
        let sleep_result = self.sleep.poll();

        match sleep_result {
            Ok(Async::Ready(())) => {
                // Timed out, send a timed out error
                Err(TimedOut.into())
            },
            _ => self.stream.poll()
        }
    }
}

//--------------------------------------------------------------//

/// `Heartbeat` that will ensure an item is yielded after at least `Duration`.
pub struct Heartbeat<S> {
    sleep: Sleep,
    stream: S
}

impl<S> Stream for Heartbeat<S> where S: Stream, S::Item: From<TimedOut> {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // Check if our sleep is up
        let sleep_result = self.sleep.poll();

        match sleep_result {
            Ok(Async::Ready(())) => {
                // Timed our, restart sleep and send a timed out object
                self.sleep.restart();

                Ok(Async::Ready(Some(TimedOut.into())))
            },
            _ => self.stream.poll()
        }
    }
}

//--------------------------------------------------------------//

/// `SleepStream` that will wait `Duration` before each yield.
pub struct SleepStream {
    sleep: Sleep
}

impl SleepStream {
    fn new(sleep: Sleep) -> SleepStream {
        SleepStream{ sleep: sleep }
    }
}

impl Stream for SleepStream {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Option<()>, ()> {
        let poll_result = self.sleep.poll();

        if let Ok(Async::Ready(())) = poll_result {
            self.sleep.restart();
        }

        poll_result.map(|async| async.map(Option::Some))
    }
}

//--------------------------------------------------------------//

/// `Sleep` future that will wait `Duration` before yielding.
pub struct Sleep {
    mapping: usize,
    duration: Duration,
    started: Instant,
    sent_task: Option<Task>,
    futures: Timer
}

impl Sleep {
    fn new(mapping: usize, duration: Duration, futures: Timer) -> Sleep {
        Sleep{ mapping: mapping, duration: duration, started: Instant::now(), sent_task: None, futures: futures }
    }

    fn restart(&mut self) {
        self.sent_task = None;
        self.started = Instant::now();
    }
}

impl Future for Sleep {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        // Check if we have hit our timeout
        if Instant::now().duration_since(self.started) >= self.duration {
            return Ok(Async::Ready(()))
        }

        // Check if we need to send a create timeout request
        let should_send_create = self.sent_task.as_ref()
            .map(|task| !task.will_notify_current())
            .unwrap_or(true);
        if should_send_create {
            // Try to queue up a create request
            let sent = self.futures.inner.try_push_create_timer(CreateTimeout{
                mapping: self.mapping, duration: self.duration, started: self.started, task: task::current() });

            // Check if we were able to send it through
            if !sent {
                warn!("Couldnt Send a Create Timeout Request From Sleep; Backing Thread May Be Running Slow");

                // Give ourselves another shot at queueing it up next time
                task::current().notify();
            } else {
                // We were able to send the reuqest, store our task, unpark the thread
                self.sent_task = Some(task::current());
                self.futures.thread.thread.unpark();
            }
        }

        Ok(Async::NotReady)
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        // Check if we should push a delete request (if we never sent the request, dont bother)
        if self.sent_task.is_some() {
            // Check if we can push a delete request (if theres room in the queue)
            let sent = self.futures.inner.try_push_delete_timer(DeleteTimeout{ mapping: self.mapping });

            if !sent {
                warn!("Couldnt Send A Delete Timeout Request From Sleep; Backing Thread May Be Running Slow");

                // Couldnt get the delete request to go through, thats fine, just make
                // sure we free our mapping, timer will lazily get cleaned up
                self.futures.inner.return_mapping(self.mapping);
            } else {
                // Delete went through, unpark the backing thread (backing thread will return our mapping)
                self.futures.thread.thread.unpark();
            }
        } else {
            // Never sent the create request, make our mapping available
            self.futures.inner.return_mapping(self.mapping);
        }
    }
}

//--------------------------------------------------------------//

// When this is dropped, that means a `Timer`s Arc<InnerTimer> has been dropped previously,
// we want the backing thread to get unparked so it can run and check if it can kill itself,
// by checking if the strong count of Arc<InnerTimer> is 1 (it is the only entity with a handle
// to it left).
struct UnparkOnDropThread {
    thread: Thread
}

impl Drop for UnparkOnDropThread {
    fn drop(&mut self) {
        self.thread.unpark();
    }
}

/// `Timer` for which different futures based timers can be created.
#[derive(Clone)]
pub struct Timer {
    inner:       Arc<InnerTimer>,
    thread:      Arc<UnparkOnDropThread>,
    max_timeout: Duration
}

impl Timer {
    /// Build a `Timer` using a `HashedWheel` internally.
    pub fn new<P>(builder: TimerBuilder, pendulum: P) -> Timer
        where P: Pendulum<TimerItem> + Send + 'static {
        let inner = Arc::new(InnerTimer::new(pendulum.max_capacity(), builder.channel_capacity()));
        let max_timeout = pendulum.max_timeout();

        let thread_inner = inner.clone();
        let thread_handle = thread::spawn(move || run_pendulum_timer(thread_inner, pendulum)).thread().clone();

        Timer{ inner: inner, thread: Arc::new(UnparkOnDropThread{ thread: thread_handle }), max_timeout: max_timeout }
    }

    /// Create a `Sleep` future that will become available after the given duration.
    pub fn sleep(&self, duration: Duration) -> PendulumResult<Sleep, ()> {
        self.validate_request(duration).map(|mapping| {
            Sleep::new(mapping, duration, self.clone())
        })
    }

    /// Create a `SleepStream` that will continuously become available for the given duration.
    pub fn sleep_stream(&self, duration: Duration) -> PendulumResult<SleepStream, ()> {
        self.sleep(duration).map(SleepStream::new)
    }

    /// Create a `Timeout` future to raise an error if the given future doesnt complete before the given duration.
    pub fn timeout<F>(&self, duration: Duration, future: F) -> PendulumResult<Timeout<F>, ()>
        where F: Future, F::Error: From<TimedOut> {
        self.sleep(duration).map(|sleep| Timeout{ opt_sleep: Some(sleep), future: future })
    }

    /// Create a `TimeoutStream` that will raise an error if an item is not yielded before the given duration.
    pub fn timeout_stream<S>(&self, duration: Duration, stream: S) -> PendulumResult<TimeoutStream<S>, ()>
        where S: Stream, S::Error: From<TimedOut> {
        self.sleep(duration).map(|sleep| TimeoutStream{ sleep: sleep, stream: stream })
    }

    /// Create a `Heartbeat` stream that will yield an item if the stream doesnt yield one before the given duration.
    pub fn heartbeat<S>(&self, duration: Duration, stream: S) -> PendulumResult<Heartbeat<S>, ()>
        where S: Stream, S::Item: From<TimedOut> {
        self.sleep(duration).map(|sleep| Heartbeat{ sleep: sleep, stream: stream })
    }

    // Validate that the timeout is valid, and see if there is room (get a mapping)
    fn validate_request(&self, duration: Duration) -> PendulumResult<usize, ()> {
        if duration > self.max_timeout {
            Err(PendulumError::new((), PendulumErrorKind::MaxCapacityReached))
        } else {
            self.inner.try_retrieve_mapping()
                .ok_or_else(|| PendulumError::new((), PendulumErrorKind::MaxTimeoutExceeded))
        }
    }
}

/// Item that the `Timer` will place inside the `Pendulum`.
/// 
/// Because type constructors are not available (yet), we have to tell users what type
/// we will be placing into the `Pendulum` that they give us. So as to not leak any
/// inner type information, we export this unconstructable type, for user type inference.
#[derive(Debug)]
pub struct TimerItem {
    task: Task,
    started: Instant,
    duration: Duration,
    mapping: usize
}

/// Run the background driver for a futures based pendulum.
/// 
/// Takes care of reading delete and create requests, as well as sending
/// notification for expired requests.
fn run_pendulum_timer<P>(inner: Arc<InnerTimer>, mut pendulum: P)
    where P: Pendulum<TimerItem> {
    let mut current_time;
    let mut last_tick_time = Instant::now();
    let mut leftover_tick = Duration::new(0, 0);

    let mut mapping_table: HashMap<usize, Token> = HashMap::with_capacity(pendulum.max_capacity());

    loop {
        current_time = Instant::now();

        // Tick the pendulum as much as we can
        // Important to do this before processing requests so that reuqests dont get any "free" ticks on them.
        let mut duration_since_last_tick = current_time.duration_since(last_tick_time) + leftover_tick; 
        while duration_since_last_tick >= pendulum.tick_duration() {
            duration_since_last_tick -= pendulum.tick_duration();

            pendulum.tick();
            last_tick_time = current_time;
        }
        // Save the leftovers...
        // TODO: Move this logic into the pendulum (provide a duration on the tick method???)
        leftover_tick = duration_since_last_tick;

        // Go through all available requests to process them
        while let Some(request) = inner.try_pop_request() {
            match request {
                TimeoutRequest::Create(create_request) => {
                    // Update our current time to guarantee it is after the started Instant of the request
                    // (Request could have been queued while we were processing other requests, after we
                    // updated our current time above)
                    current_time = Instant::now();

                    let time_to_schedule = current_time.duration_since(create_request.started);
                    let real_timeout = create_request.duration.checked_sub(time_to_schedule).unwrap_or(Duration::new(0, 0));

                    if real_timeout == Duration::new(0, 0) {
                        create_request.task.notify()
                    } else {
                        // Add in the duration since last tick, in case it took a while to get here after updating our tick
                        // Could still be the case that Instant jumps forward before we can insert our timeout, we handle
                        // re-queueing in our expiration logic as an edge case, this addition to the timeout just reduces
                        // the chance for that edge case
                        duration_since_last_tick = current_time.duration_since(last_tick_time) + leftover_tick; 
                        let accurate_real_timeout = real_timeout + duration_since_last_tick;

                        // Push the request onto the pendulum, client should have taken care of checking for max
                        // timeout, and they got a token, so we know we should have capacity for the timer
                        let item = TimerItem{ task: create_request.task, started: create_request.started,
                            duration: create_request.duration, mapping: create_request.mapping };
                        let token = pendulum.insert_timeout(accurate_real_timeout, item)
                            .expect("pendulum: Failed To Push Timeout Onto Pendulum");

                        // Push the mapping to our table
                        // Dont panic if a mapping already existed in the table, client wasnt able to
                        // push a delete request which is fine (or they updated their Task object!!!)
                        mapping_table.insert(create_request.mapping, token);
                    }
                },
                TimeoutRequest::Delete(delete_request) => {
                    // Remove the mapping from our table
                    let mapping = delete_request.mapping;

                    // If we had a mapping, then remove it, otherwise, scheduling time may have taken up their full timeout,
                    // in which case, we notified them immediately and never inserted the timeout into the pendulum
                    if let Some(token) = mapping_table.remove(&mapping) {
                        // If a client went to delete the request, and they pushed into the delete queue, but then
                        // the backing timer saw that the timeout was triggered, then this timeout may not be in
                        // the pendulum anymore, so thats fine, dont unrwap here
                        pendulum.remove_timeout(token);
                    }

                    // Push the mapping back to the queue so someone else can use it; if the client wasnt able to
                    // push to the delete queue because it was full, they would have pushed the mapping back themselves
                    inner.return_mapping(delete_request.mapping);
                }
            }
        }

        // Update current time for expiration checking
        current_time = Instant::now();

        // Expire as many timeouts as we can
        while let Some(TimerItem{ task, started, duration, mapping }) = pendulum.expired_timeout() {
            let total_time = current_time.duration_since(started);

            if total_time < duration {
                let requeue_duration = duration - total_time;

                // Edge case: Task is not really finished, time between updating our ticks,
                // and queueing requests was too far apart, re-queue with the difference
                warn!("Task Was Ready Before Duration Of {:?} Was Up, Leftover Duration Was {:?}; Re-Queueing", duration, requeue_duration);

                let item = TimerItem{ task: task, started: started, duration: duration, mapping: mapping };
                let token = pendulum.insert_timeout(requeue_duration, item)
                    .expect("pendulum: Failed To Re-Push Timeout Onto Pendulum");

                mapping_table.insert(mapping, token);
            } else {
                // Task is really finished, notify them
                task.notify();
            }
        }
        
        // If all other instances of Arc<InnerTimer> were dropped, we can shutdown
        if Arc::strong_count(&inner) == 1 {
            break;
        } else {
            // Park the thread until we think we can make another tick on our pendulum (or the client unparks us)
            let time_to_next_tick = pendulum.tick_duration() - leftover_tick;
            thread::park_timeout(time_to_next_tick);
        }
    }
}

//--------------------------------------------------------------//

enum TimeoutRequest {
    Create(CreateTimeout),
    Delete(DeleteTimeout)
}

struct CreateTimeout {
    mapping: usize,
    duration: Duration,
    started: Instant,
    task: Task
}

struct DeleteTimeout {
    mapping: usize
}

struct InnerTimer {
    // Use case here is three fold, if a client was able to get a mapping, then they guarantee themselves
    // an entry in our timer wheel. Also, the backing thread will associate the mapping with the actual
    // token associated with a timer, so we dont have to know what that token is. Additionally, if we want
    // to delete the timer, but our delete queue is full, we can just push the mapping on the queue, forget
    // about deleting it, and when either the timer expires, or someone else comes along to re-use that
    // mapping, the timer wheel thread will remove the token associated with the mapping first (if it exists).
    mapping_queue: SegQueue<usize>,
    // We CANT separate this our to a create queue and delete queue, because there is a case where, we are processing create
    // requests happening AFTER processing delete requests, and a client pushes a delete request 
    // Tried seperating out to a create queue and delete queue, the problem there is if a client pushes to the create
    // queue, then later expires and pushes to the delete queue (because it knows it already created the timer), since
    // our backing thread HAS to process delete requests before create requests (due to the fact that if a client has
    // a mapping, they are guaranteed a slot, so we have to clear space first), we cant make this two queues, otherwise,
    // we risk delete messages being seen for non-existant create requests, causing inconsistencies (and panics for small
    // timeouts!)
    request_queue:  (SegQueue<TimeoutRequest>, AtomicUsize),
    channel_capacity: usize
}

impl InnerTimer {
    /// Create a new `InnerTimer`.
    pub fn new(timer_capacity: usize, channel_capacity: usize) -> InnerTimer {
        let mapping_queue = SegQueue::new();

        // Generate a bunch of mappings that clients can use as proxy `Token`s
        let mut next_mapping = 0;
        for _ in 0..timer_capacity {
            mapping_queue.push(next_mapping);

            next_mapping += 1;
        }

        InnerTimer{ mapping_queue: mapping_queue, request_queue: (SegQueue::new(), AtomicUsize::new(0)),
            channel_capacity: channel_capacity }
    }

    // Channel capacity for any created or deleted requests.
    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity
    }

    /// Attempt to retrieve a mapping, which is required for creating/deleting a timer.
    pub fn try_retrieve_mapping(&self) -> Option<usize> {
        self.mapping_queue.try_pop()
    }

    /// Clients should only call this if they were unable to issue a delete timer request.
    /// 
    /// Backing thread should call this after processing a delete timer request.
    pub fn return_mapping(&self, mapping: usize) {
        self.mapping_queue.push(mapping);
    }

    /// Attempt to enqueue a create timer request.
    pub fn try_push_create_timer(&self, timer: CreateTimeout) -> bool {
        try_push(&self.request_queue.0, &self.request_queue.1, self.channel_capacity(), TimeoutRequest::Create(timer))
    }

    /// Attempt to enqueue a delete timer request.
    pub fn try_push_delete_timer(&self, timer: DeleteTimeout) -> bool {
        try_push(&self.request_queue.0, &self.request_queue.1, self.channel_capacity(), TimeoutRequest::Delete(timer))
    }

    /// Attempt to dequeue a timer request.
    pub fn try_pop_request(&self) -> Option<TimeoutRequest> {
        self.request_queue.0.try_pop().map(|request| {
            self.request_queue.1.fetch_sub(1, Ordering::AcqRel);
            request
        })
    }
}

/// Attempt to push an item onto the queue, using the given atomic length and max capacity.
fn try_push<T>(queue: &SegQueue<T>, len: &AtomicUsize, capacity: usize, item: T) -> bool {
        let queue_size = len.fetch_add(1, Ordering::AcqRel);

        if queue_size >= capacity {
            len.fetch_sub(1, Ordering::AcqRel);
            false
        } else {
            queue.push(item);
            true
        }
}

//--------------------------------------------------------------//

#[cfg(test)]
mod tests {
    use super::{TimerBuilder, TimeoutStatus};

    use wheel::HashedWheelBuilder;

    use std::time::{Duration};
    use std::sync::Arc;
    use std::mem;
    use std::thread;

    use futures::{Future, Stream};
    use futures::sync::mpsc::{self, UnboundedReceiver};
    use futures::future;

    #[test]
    fn positive_thread_shutdown_with_dropped_timer() {
        // Create a timer with a large tick interval
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default()
                .with_tick_duration(Duration::from_millis(10000))
                .build()
            );

        assert_eq!(2, Arc::strong_count(&timer.inner));

        // Grab a weak reference to the Arc
        let weak = Arc::downgrade(&timer.inner);

        // Force the backing thread to run at least once
        thread::sleep(Duration::from_millis(50));
        // Explicitly drop the timer
        mem::drop(timer);
        // Give the background thread time to realize the drop
        thread::sleep(Duration::from_millis(50));

        // Assert that we cant upgrade the weak (thread shutdown)
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn positive_thread_shutdown_with_dropped_clone() {
        let timer_one = TimerBuilder::default()
            .build(HashedWheelBuilder::default()
                .with_tick_duration(Duration::from_millis(10000))
                .build()
            );
        let timer_two = timer_one.clone();

        assert_eq!(3, Arc::strong_count(&timer_one.inner));

        let weak = Arc::downgrade(&timer_one.inner);

        thread::sleep(Duration::from_millis(50));
        mem::drop(timer_one);
        mem::drop(timer_two);
        thread::sleep(Duration::from_millis(50));

        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn positive_thread_shutdown_with_dropped_sleep() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());

        let sleep = timer.sleep(Duration::from_millis(100))
            .unwrap();

        let weak = Arc::downgrade(&timer.inner);

        thread::sleep(Duration::from_millis(50));
        mem::drop(timer);
        mem::drop(sleep);
        thread::sleep(Duration::from_millis(50));

        // Assert that we cant upgrade the weak
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn positive_sleep_wakes_on_milli() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let sleep = timer
            .sleep(Duration::from_millis(50))
            .unwrap();
    
        sleep.wait().unwrap();
    }

    #[test]
    fn positive_sleep_wakes_on_nano() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let sleep = timer
            .sleep(Duration::new(0, 1))
            .unwrap();
    
        sleep.wait().unwrap();
    }

    #[test]
    fn positive_sleep_wakes_on_zero() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let sleep = timer
            .sleep(Duration::new(0, 0))
            .unwrap();
    
        sleep.wait().unwrap();
    }

    #[test]
    fn positive_sleep_stream_yields_twice() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let mut stream = timer
            .sleep_stream(Duration::from_millis(50))
            .unwrap()
            .wait();

        stream.next().unwrap().unwrap();
        stream.next().unwrap().unwrap();
    }

    #[test]
    fn positive_heartbeat_sends_timeout() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let (_send, recv): (_, UnboundedReceiver<TimeoutStatus<()>>) = mpsc::unbounded();
        let mut stream = timer
            .heartbeat(Duration::from_millis(50), recv)
            .unwrap()
            .wait();

        assert_eq!(TimeoutStatus::TimedOut, stream.next().unwrap().unwrap());
        assert_eq!(TimeoutStatus::TimedOut, stream.next().unwrap().unwrap());
    }

    #[test]
    fn positive_heartbeat_sends_item() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let (send, recv): (_, UnboundedReceiver<TimeoutStatus<()>>) = mpsc::unbounded();
        send.unbounded_send(TimeoutStatus::Original(())).unwrap();
        send.unbounded_send(TimeoutStatus::Original(())).unwrap();

        let mut stream = timer
            .heartbeat(Duration::from_millis(50), recv)
            .unwrap()
            .wait();

        assert_eq!(TimeoutStatus::Original(()), stream.next().unwrap().unwrap());
        assert_eq!(TimeoutStatus::Original(()), stream.next().unwrap().unwrap());
    }

    #[test]
    fn positive_heartbeat_send_item_and_timeout() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let (send, recv): (_, UnboundedReceiver<TimeoutStatus<()>>) = mpsc::unbounded();
        send.unbounded_send(TimeoutStatus::Original(())).unwrap();
        send.unbounded_send(TimeoutStatus::Original(())).unwrap();

        let mut stream = timer
            .heartbeat(Duration::from_millis(50), recv)
            .unwrap()
            .wait();

        assert_eq!(TimeoutStatus::Original(()), stream.next().unwrap().unwrap());
        assert_eq!(TimeoutStatus::Original(()), stream.next().unwrap().unwrap());
        assert_eq!(TimeoutStatus::TimedOut, stream.next().unwrap().unwrap());
    }

    #[test]
    fn positive_timeout_times_out() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let result = timer
            .timeout(Duration::from_millis(50), future::empty::<(), TimeoutStatus<()>>())
            .unwrap()
            .wait();

        assert_eq!(TimeoutStatus::TimedOut, result.unwrap_err());
    }

    #[test]
    fn positive_timeout_stream_times_out() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default().build());
        let (_send, recv): (_, UnboundedReceiver<()>) = mpsc::unbounded();
        let mut stream = timer
            .timeout_stream(Duration::from_millis(50), recv.map_err(TimeoutStatus::Original))
            .unwrap()
            .wait();

        assert_eq!(TimeoutStatus::TimedOut, stream.next().unwrap().unwrap_err());
        assert_eq!(TimeoutStatus::TimedOut, stream.next().unwrap().unwrap_err());
    }

    #[test]
    fn negative_thread_shutdown_with_existing_sleep() {
        let timer = TimerBuilder::default()
            .build(HashedWheelBuilder::default()
                .with_tick_duration(Duration::from_millis(10000))
                .build()
            );
        let sleep = timer.sleep(Duration::from_millis(100))
            .unwrap();

        assert_eq!(3, Arc::strong_count(&timer.inner));

        thread::sleep(Duration::from_millis(50));
        mem::drop(timer);
        thread::sleep(Duration::from_millis(50));

        assert_eq!(2, Arc::strong_count(&sleep.futures.inner));
    }
}