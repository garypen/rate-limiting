#[cfg(test)]
mod python_integration_tests {
    use std::process::Command;

    #[test]
    fn run_python_tests() {
        // Paths are relative to the shot-limit directory, where the test is executed.
        let venv_path = ".venv";
        let python_executable = format!("{}/bin/python", venv_path);
        let pip_executable = format!("{}/bin/pip", venv_path);
        let maturin_executable = format!("{}/bin/maturin", venv_path);
        let test_py_path = "test.py";

        // Step 1: Create a Python virtual environment if it doesn't exist
        println!("Creating Python virtual environment...");
        let venv_status = Command::new("python3")
            .arg("-m")
            .arg("venv")
            .arg(venv_path)
            .status()
            .expect("Failed to execute python3 venv command");

        assert!(venv_status.success(), "Failed to create virtual environment");

        // Step 2: Install maturin into the virtual environment
        let pip_install_maturin_status = Command::new(&pip_executable)
            .arg("install")
            .arg("maturin")
            .status()
            .expect("Failed to execute pip install maturin command");

        assert!(pip_install_maturin_status.success(), "Failed to install maturin into virtual environment");

        // Step 3: Build and install the Python package using maturin
        let maturin_status = Command::new(&maturin_executable)
            .arg("develop")
            .arg("--release")
            .status()
            .expect("Failed to execute maturin command.");

        assert!(maturin_status.success(), "Failed to build and install Python package with maturin");

        // Step 4: Run the Python tests within the activated virtual environment
        let python_test_status = Command::new(&python_executable)
            .arg(test_py_path)
            .status()
            .expect("Failed to execute python test.py command");

        assert!(python_test_status.success(), "Python tests failed");

        println!("Python tests completed successfully.");
    }
}
