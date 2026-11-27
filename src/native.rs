use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::board;

#[pyfunction]
fn validate(graph: &str) -> PyResult<()> {
    board::validate(graph).map_err(|err| PyValueError::new_err(err.to_string()))
}

#[pyfunction]
fn run(graph: &str, events: u64) -> PyResult<String> {
    board::run(graph, events).map_err(|err| PyValueError::new_err(err.to_string()))
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    module.add_function(wrap_pyfunction!(run, module)?)?;
    Ok(())
}
