#![feature(test)]

extern crate pendulum;
extern crate test;

#[cfg(test)]
mod benches {
    mod pendulum {
        use std::time::Duration;

        use pendulum::{Pendulum, HashedWheelBuilder};
        use test::Bencher;

        #[bench]
        fn bench_insert_single_one_second_timeout(b: &mut Bencher) {
            let mut wheel = HashedWheelBuilder::default()
                .build();

            b.iter(|| {
                let token = wheel.insert_timeout(Duration::new(1, 0), ()).unwrap();
                wheel.remove_timeout(token).unwrap();
            });
        }

        #[bench]
        fn bench_insert_million_one_second_timeouts(b: &mut Bencher) {
            let mut wheel = HashedWheelBuilder::default()
                .with_init_capacity(1_000_000)
                .build();

            b.iter(|| {
                for _ in 0..1_000_000 {
                    let token = wheel.insert_timeout(Duration::new(1, 0), ()).unwrap();
                    wheel.remove_timeout(token).unwrap();
                }
            });
        }

        #[bench]
        fn bench_expire_million_one_second_timeouts(b: &mut Bencher) {
            let mut wheel = HashedWheelBuilder::default()
                .with_tick_duration(Duration::from_millis(100))
                .with_num_slots(11)
                .with_init_capacity(1_000_000)
                .with_max_capacity(1_000_000)
                .build();

            for _ in 0..1_000_000 {
                wheel.insert_timeout(Duration::new(1, 0), ()).unwrap();
            }

            for _ in 0..11 {
                wheel.tick();
            }

            let mut timers_expired = 0;
            b.iter(|| {
                while timers_expired < 1_000_000 {
                    if wheel.expired_timeout().is_some() {
                        timers_expired += 1;
                    }
                }
            });
        }
    }
}