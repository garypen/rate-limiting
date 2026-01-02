use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use shot_limit::FixedWindow;
use shot_limit::SlidingWindow;
use shot_limit::Strategy;
use shot_limit::TokenBucket;

fn bench_single_strategy<S: Strategy>(group_name: &str, c: &mut Criterion, strategy: Arc<S>) {
    let mut group = c.benchmark_group(group_name);

    group.bench_function("single-threaded", |b| {
        b.iter(|| {
            let _ = black_box(&strategy).process();
        })
    });

    group.finish();
}

fn bench_parallel_strategy<S: Strategy + Send + Sync + 'static>(
    group_name: &str,
    c: &mut Criterion,
    strategy: Arc<S>,
) {
    let strategy = Arc::new(strategy);
    let mut group = c.benchmark_group(group_name);

    for threads in [2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}-threads", threads)),
            threads,
            |b, &num_threads| {
                b.iter_custom(|iters| {
                    let mut handles = Vec::with_capacity(num_threads);
                    let start = Instant::now();

                    for _ in 0..num_threads {
                        let s = Arc::clone(&strategy);
                        let iters_per_thread = iters / num_threads as u64;

                        handles.push(thread::spawn(move || {
                            for _ in 0..iters_per_thread {
                                let _ = black_box(s.process());
                            }
                        }));
                    }

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

fn run_all_benches(c: &mut Criterion) {
    let limit = NonZeroUsize::new(1_000_000).unwrap();
    let period = Duration::from_secs(60);

    // FixedWindow
    let fw = Arc::new(FixedWindow::new(limit, period));
    bench_single_strategy("Baseline FixedWindow", c, Arc::clone(&fw));
    bench_parallel_strategy("Parallel FixedWindow", c, fw);

    // SlidingWindow
    let sw = Arc::new(SlidingWindow::new(limit, period));
    bench_single_strategy("Baseline SlidingWindow", c, Arc::clone(&sw));
    bench_parallel_strategy("Parallel SlidingWindow", c, sw);

    // TokenBucket
    let tb = Arc::new(TokenBucket::new(limit, limit, period));
    bench_single_strategy("Baseline TokenBucket", c, Arc::clone(&tb));
    bench_parallel_strategy("Parallel TokenBucket", c, tb);
}

criterion_group!(benches, run_all_benches);
criterion_main!(benches);
