#![feature(test)]

extern crate pendulum;
extern crate test;

#[cfg(test)]
mod benches {
    mod pendulum {
        use std::time::Duration;

        use pendulum::{PendulumBuilder};
        use test::Bencher;

        #[bench]
        fn bench_insert_million_one_second_timeouts(b: &mut Bencher) {
            let mut pendulum = PendulumBuilder::default()
                .with_init_capacity(1_000_000)
                .build();

            b.iter(|| {
                for _ in 0..1_000_000 {
                    let token = pendulum.insert_timeout(Duration::new(1, 0), ()).unwrap();
                    pendulum.remove_timeout(token).unwrap();
                }
            });
        }

        #[bench]
        fn bench_expire_million_one_second_timeouts(b: &mut Bencher) {
            let mut pendulum = PendulumBuilder::default()
                .with_tick_duration(Duration::from_millis(100))
                .with_num_slots(11)
                .with_init_capacity(1_000_000)
                .with_max_capacity(1_000_000)
                .build();

            for _ in 0..1_000_000 {
                pendulum.insert_timeout(Duration::new(1, 0), ()).unwrap();
            }

            for _ in 0..11 {
                pendulum.ticker().tick();
            }

            let mut timers_expired = 0;
            b.iter(|| {
                while timers_expired < 1_000_000 {
                    if pendulum.expired_timeout().is_some() {
                        timers_expired += 1;
                    }
                }
            });
        }
    }
}