use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::Barrier;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;

use governor::Quota;
use governor::RateLimiter;
use governor::clock::Clock;
use governor::clock::QuantaClock;
use governor::state::InMemoryState;
use governor::state::direct::NotKeyed;

use shot_limit::FixedWindow;
use shot_limit::Gcra;
use shot_limit::Reason;
use shot_limit::SlidingWindow;
use shot_limit::Strategy;
use shot_limit::TokenBucket;

// Wrapper to bridge Governor into the shot-limit Strategy trait
#[derive(Debug)]
struct GovernorStrategy {
    limiter: Arc<RateLimiter<NotKeyed, InMemoryState, QuantaClock>>,
    clock: QuantaClock,
}

impl Strategy for GovernorStrategy {
    fn process(&self) -> ControlFlow<Reason> {
        match self.limiter.check() {
            Ok(_) => ControlFlow::Continue(()),
            Err(negative) => {
                let now = self.clock.now();
                let wait: Duration = negative.wait_time_from(now);
                ControlFlow::Break(Reason::Overloaded { retry_after: wait })
            }
        }
    }
}

fn bench_single_strategy<S: Strategy>(group_name: &str, c: &mut Criterion, strategy: Arc<S>) {
    let mut group = c.benchmark_group(group_name);

    group.bench_function("single-threaded", |b| {
        b.iter(|| {
            let _ = black_box(strategy.as_ref()).process();
        })
    });

    group.finish();
}

fn bench_parallel_strategy<S: Strategy + Send + Sync + 'static>(
    group_name: &str,
    c: &mut Criterion,
    strategy: Arc<S>,
) {
    let mut group = c.benchmark_group(group_name);

    for threads in [2, 4, 8].iter() {
        let num_threads = *threads;
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}-threads", num_threads)),
            &num_threads,
            |b, &n| {
                b.iter_custom(|iters| {
                    let barrier = Arc::new(Barrier::new(n + 1));
                    let mut handles = Vec::with_capacity(n);

                    for _ in 0..n {
                        let s = Arc::clone(&strategy);
                        let bar = Arc::clone(&barrier);
                        let iters_per_thread = iters / n as u64;

                        handles.push(thread::spawn(move || {
                            bar.wait(); // Wait for the start signal
                            for _ in 0..iters_per_thread {
                                let _ = black_box(s.process());
                            }
                        }));
                    }

                    // Synchronize the start across all threads
                    barrier.wait();
                    let start = Instant::now();

                    for handle in handles {
                        let _ = handle.join();
                    }

                    start.elapsed()
                });
            },
        );
    }
    group.finish();
}

fn bench_dynamic_strategy(
    group_name: &str,
    c: &mut Criterion,
    strategy: Arc<dyn Strategy + Send + Sync>,
) {
    let mut group = c.benchmark_group(format!("Dynamic-{}", group_name));

    group.bench_function("single-threaded", |b| {
        b.iter(|| {
            let _ = black_box(strategy.as_ref()).process();
        })
    });

    group.finish();
}

fn run_all_benches(c: &mut Criterion) {
    let limit_val = 1_000_000;
    let limit = NonZeroUsize::new(limit_val).unwrap();
    let period = Duration::from_secs(60);

    // --- 1. Initialize all strategies ---

    let fw = Arc::new(FixedWindow::new(limit, period));
    let sw = Arc::new(SlidingWindow::new(limit, period));
    let tb = Arc::new(TokenBucket::new(limit, limit, period));
    let gcra = Arc::new(Gcra::new(limit, period));

    // Governor setup
    let gov_quota = Quota::per_minute(NonZeroU32::new(limit_val as u32).unwrap());
    let gov_clock = QuantaClock::default();
    let gov_limiter = Arc::new(RateLimiter::direct_with_clock(gov_quota, gov_clock.clone()));
    let gov = Arc::new(GovernorStrategy {
        limiter: gov_limiter,
        clock: gov_clock,
    });

    // --- 2. Run Static Dispatch Benches (Direct calls) ---

    // FixedWindow
    bench_single_strategy("FixedWindow-Static", c, Arc::clone(&fw));
    bench_parallel_strategy("FixedWindow-Static", c, fw.clone());

    // SlidingWindow
    bench_single_strategy("SlidingWindow-Static", c, Arc::clone(&sw));
    bench_parallel_strategy("SlidingWindow-Static", c, sw.clone());

    // TokenBucket
    bench_single_strategy("TokenBucket-Static", c, Arc::clone(&tb));
    bench_parallel_strategy("TokenBucket-Static", c, tb.clone());

    // Gcra
    bench_single_strategy("Gcra-Static", c, Arc::clone(&gcra));
    bench_parallel_strategy("Gcra-Static", c, gcra.clone());

    // Governor
    bench_single_strategy("Governor-Static", c, Arc::clone(&gov));
    bench_parallel_strategy("Governor-Static", c, gov.clone());

    // --- 3. Run Dynamic Dispatch Benches (Trait Objects) ---
    // This allows us to see the overhead of Arc<dyn Strategy>

    let strategies: Vec<(&str, Arc<dyn Strategy + Send + Sync>)> = vec![
        ("FixedWindow", fw),
        ("SlidingWindow", sw),
        ("TokenBucket", tb),
        ("Gcra", gcra),
        ("Governor", gov),
    ];

    for (name, strategy) in strategies {
        bench_dynamic_strategy(name, c, strategy);
    }
}

/*
fn run_all_benches(c: &mut Criterion) {
    let limit_val = 1_000_000;
    let limit = NonZeroUsize::new(limit_val).unwrap();
    let period = Duration::from_secs(60);

    // 1. FixedWindow
    let fw = Arc::new(FixedWindow::new(limit, period));
    bench_single_strategy("FixedWindow", c, Arc::clone(&fw));
    bench_parallel_strategy("FixedWindow", c, fw);

    // 2. SlidingWindow
    let sw = Arc::new(SlidingWindow::new(limit, period));
    bench_single_strategy("SlidingWindow", c, Arc::clone(&sw));
    bench_parallel_strategy("SlidingWindow", c, sw);

    // 3. TokenBucket
    let tb = Arc::new(TokenBucket::new(limit, limit, period));
    bench_single_strategy("TokenBucket", c, Arc::clone(&tb));
    bench_parallel_strategy("TokenBucket", c, tb);

    // 4. Gcra
    let gcra = Arc::new(Gcra::new(limit, period));
    bench_single_strategy("Gcra", c, Arc::clone(&gcra));
    bench_parallel_strategy("Gcra", c, gcra.clone());

    // 5. Governor (Corrected Initialization)
    let gov_quota = Quota::per_minute(NonZeroU32::new(limit_val as u32).unwrap());
    let gov_clock = QuantaClock::default();

    // Pass clones of the clock to avoid ownership/reference issues
    let gov_limiter = Arc::new(RateLimiter::direct_with_clock(gov_quota, gov_clock.clone()));
    let gov_strat = Arc::new(GovernorStrategy {
        limiter: gov_limiter,
        clock: gov_clock,
    });

    bench_single_strategy("Governor", c, Arc::clone(&gov_strat));
    bench_parallel_strategy("Governor", c, gov_strat);

    // Dynamic dispatch test (Interface Tax)
    bench_dynamic_strategy("Gcra", c, gcra);
}
fn run_all_benches(c: &mut Criterion) {
    let limit_val = 1_000_000;
    let limit = NonZeroUsize::new(limit_val).unwrap();
    let period = Duration::from_secs(60);

    // FixedWindow
    let fw = Arc::new(FixedWindow::new(limit, period));
    bench_single_strategy("FixedWindow", c, Arc::clone(&fw));
    bench_parallel_strategy("FixedWindow", c, fw);

    // SlidingWindow
    let sw = Arc::new(SlidingWindow::new(limit, period));
    bench_single_strategy("SlidingWindow", c, Arc::clone(&sw));
    bench_parallel_strategy("SlidingWindow", c, sw);

    // TokenBucket
    let tb = Arc::new(TokenBucket::new(limit, limit, period));
    bench_single_strategy("TokenBucket", c, Arc::clone(&tb));
    bench_parallel_strategy("TokenBucket", c, tb);

    // Gcra
    let gcra = Arc::new(Gcra::new(limit, period));
    bench_single_strategy("Gcra", c, Arc::clone(&gcra));
    bench_parallel_strategy("Gcra", c, gcra.clone());

    // Governor (Integrated into standard strategy benches)
    let gov_quota = Quota::per_minute(NonZeroU32::new(limit_val as u32).unwrap());
    let gov_clock = QuantaClock::default();
    let gov_limiter = Arc::new(RateLimiter::direct_with_clock(gov_quota, &gov_clock));
    let gov_strat = Arc::new(GovernorStrategy {
        limiter: gov_limiter,
        clock: gov_clock,
    });

    bench_single_strategy("Governor", c, Arc::clone(&gov_strat));
    bench_parallel_strategy("Governor", c, gov_strat.clone());

    // Dynamic dispatch test to see the "Interface Tax"
    bench_dynamic_strategy("Gcra", c, gcra);
}
*/

criterion_group!(benches, run_all_benches);
criterion_main!(benches);
