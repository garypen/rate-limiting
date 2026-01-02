use std::fmt::Debug;
use std::ops::ControlFlow;
use std::time::Duration;

mod fixed_window;
mod sliding_window;
mod token_bucket;

pub use fixed_window::FixedWindow;
pub use sliding_window::SlidingWindow;
pub use token_bucket::TokenBucket;

#[derive(Debug, PartialEq)]
pub enum Reason {
    Overloaded { retry_after: Duration },
}

pub trait Strategy: Debug {
    fn process(&self) -> ControlFlow<Reason>;
}
