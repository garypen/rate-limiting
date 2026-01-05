//! # py-shot-limit
//!
//! `py-shot-limit` a Python wrapper for `shot-limit`.

#[cfg(test)]
mod python_tests;

mod python;

#[pyo3::prelude::pymodule]
fn py_shot_limit(
    m: pyo3::prelude::Bound<'_, pyo3::prelude::PyModule>,
) -> pyo3::prelude::PyResult<()> {
    python::init_python_module(&m)?;
    Ok(())
}
