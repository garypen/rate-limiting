use std::num::NonZeroUsize;
use std::time::Duration;

use pyo3::prelude::*;

use crate::{FixedWindow, Gcra, SlidingWindow, Strategy, TokenBucket};

/// A dummy function to verify the Python bindings.
#[pyfunction]
fn hello() -> PyResult<String> {
    Ok("Hello from shot-limit!".to_string())
}

#[pyclass(name = "TokenBucket")]
struct PyTokenBucket(TokenBucket);

#[pymethods]
impl PyTokenBucket {
    #[new]
    fn new(capacity: usize, increment: usize, period_secs: u64) -> PyResult<Self> {
        let capacity = NonZeroUsize::new(capacity).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>("capacity must be non-zero")
        })?;
        let increment = NonZeroUsize::new(increment).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>("increment must be non-zero")
        })?;
        let period = Duration::from_secs(period_secs);
        Ok(PyTokenBucket(TokenBucket::new(capacity, increment, period)))
    }

    fn process(&self) -> bool {
        self.0.process().is_continue()
    }
}

#[pyclass(name = "FixedWindow")]
struct PyFixedWindow(FixedWindow);

#[pymethods]
impl PyFixedWindow {
    #[new]
    fn new(capacity: usize, interval_secs: u64) -> PyResult<Self> {
        let capacity = NonZeroUsize::new(capacity).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>("capacity must be non-zero")
        })?;
        let interval = Duration::from_secs(interval_secs);
        Ok(PyFixedWindow(FixedWindow::new(capacity, interval)))
    }

    fn process(&self) -> bool {
        self.0.process().is_continue()
    }
}

#[pyclass(name = "Gcra")]
struct PyGcra(Gcra);

#[pymethods]
impl PyGcra {
    #[new]
    fn new(limit: usize, period_secs: u64) -> PyResult<Self> {
        let limit = NonZeroUsize::new(limit).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>("limit must be non-zero")
        })?;
        let period = Duration::from_secs(period_secs);
        Ok(PyGcra(Gcra::new(limit, period)))
    }

    fn process(&self) -> bool {
        self.0.process().is_continue()
    }
}

#[pyclass(name = "SlidingWindow")]
struct PySlidingWindow(SlidingWindow);

#[pymethods]
impl PySlidingWindow {
    #[new]
    fn new(capacity: usize, interval_secs: u64) -> PyResult<Self> {
        let capacity = NonZeroUsize::new(capacity).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>("capacity must be non-zero")
        })?;
        let interval = Duration::from_secs(interval_secs);
        Ok(PySlidingWindow(SlidingWindow::new(capacity, interval)))
    }

    fn process(&self) -> bool {
        self.0.process().is_continue()
    }
}

pub fn init_python_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_class::<PyTokenBucket>()?;
    m.add_class::<PyFixedWindow>()?;
    m.add_class::<PyGcra>()?;
    m.add_class::<PySlidingWindow>()?;
    Ok(())
}
