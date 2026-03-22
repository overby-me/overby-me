/// CUDA helpers module: min_driver_version, nvcc_arch_flags, nvcc_arch_readable.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "cuda.min_driver_version".to_string()),
        cuda_min_driver_version,
    );
    vm.method_registry.insert(
        ("module".to_string(), "cuda.nvcc_arch_flags".to_string()),
        cuda_nvcc_arch_flags,
    );
    vm.method_registry.insert(
        ("module".to_string(), "cuda.nvcc_arch_readable".to_string()),
        cuda_nvcc_arch_readable,
    );
}

fn cuda_min_driver_version(
    _vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let cuda_version = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => {
            return Err(
                "cuda.min_driver_version: first argument must be CUDA version string".to_string(),
            );
        }
    };

    // Map CUDA toolkit version to minimum driver version
    // Based on NVIDIA's compatibility matrix
    let driver = match cuda_version.as_str() {
        "12.4" | "12.4.0" => "550.54.14",
        "12.3" | "12.3.0" | "12.3.1" | "12.3.2" => "545.23.06",
        "12.2" | "12.2.0" | "12.2.1" | "12.2.2" => "535.54.03",
        "12.1" | "12.1.0" | "12.1.1" => "530.30.02",
        "12.0" | "12.0.0" | "12.0.1" => "525.60.13",
        "11.8" | "11.8.0" => "520.61.05",
        "11.7" | "11.7.0" | "11.7.1" => "515.43.04",
        "11.6" | "11.6.0" | "11.6.1" | "11.6.2" => "510.39.01",
        "11.5" | "11.5.0" | "11.5.1" | "11.5.2" => "495.29.05",
        "11.4" | "11.4.0" | "11.4.1" | "11.4.2" | "11.4.3" | "11.4.4" => "470.42.01",
        "11.3" | "11.3.0" | "11.3.1" => "465.19.01",
        "11.2" | "11.2.0" | "11.2.1" | "11.2.2" => "460.27.04",
        "11.1" | "11.1.0" | "11.1.1" => "455.23",
        "11.0" | "11.0.1" | "11.0.2" | "11.0.3" => "450.36.06",
        "10.2" | "10.2.89" => "440.33",
        "10.1" | "10.1.105" | "10.1.168" | "10.1.243" => "418.39",
        "10.0" | "10.0.130" => "410.48",
        "9.2" | "9.2.88" | "9.2.148" => "396.26",
        "9.1" | "9.1.85" => "390.46",
        "9.0" | "9.0.76" => "384.81",
        "8.0" | "8.0.44" | "8.0.61" => "367.48",
        _ => ">=525.60.13", // Default to recent
    };

    Ok(Object::String(driver.to_string()))
}

fn cuda_nvcc_arch_flags(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let cuda_version = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        Some(Object::Compiler(_)) => {
            // If passed a compiler object, extract version
            "12.0".to_string()
        }
        _ => {
            return Err(
                "cuda.nvcc_arch_flags: first argument must be CUDA version or compiler".to_string(),
            );
        }
    };

    // Get requested architectures (or 'Auto')
    let arches: Vec<String> = positional
        .iter()
        .skip(1)
        .filter_map(|o| {
            if let Object::String(s) = o {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    let _detected = VM::get_arg_bool(args, "detected", true);

    let arch_list: Vec<String> =
        if arches.is_empty() || arches.iter().any(|a| a == "Auto" || a == "auto") {
            let major: u32 = cuda_version
                .split('.')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(11);

            if major >= 12 {
                vec![
                    "5.0", "5.2", "6.0", "6.1", "7.0", "7.5", "8.0", "8.6", "8.9", "9.0",
                ]
            } else if major >= 11 {
                vec!["5.0", "5.2", "6.0", "6.1", "7.0", "7.5", "8.0", "8.6"]
            } else {
                vec!["3.0", "3.5", "5.0", "5.2", "6.0", "6.1", "7.0", "7.5"]
            }
            .into_iter()
            .map(|s| s.to_string())
            .collect()
        } else {
            arches
        };

    let flags: Vec<Object> = arch_list
        .iter()
        .map(|arch| {
            let sm = arch.replace('.', "");
            Object::String(format!("-gencode=arch=compute_{},code=sm_{}", sm, sm))
        })
        .collect();

    Ok(Object::Array(flags))
}

fn cuda_nvcc_arch_readable(
    _vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let cuda_version = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        Some(Object::Compiler(_)) => "12.0".to_string(),
        _ => {
            return Err(
                "cuda.nvcc_arch_readable: first argument must be CUDA version or compiler"
                    .to_string(),
            );
        }
    };

    let arches: Vec<String> = positional
        .iter()
        .skip(1)
        .filter_map(|o| {
            if let Object::String(s) = o {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    let arch_list: Vec<String> =
        if arches.is_empty() || arches.iter().any(|a| a == "Auto" || a == "auto") {
            let major: u32 = cuda_version
                .split('.')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(11);
            if major >= 12 {
                vec![
                    "5.0", "5.2", "6.0", "6.1", "7.0", "7.5", "8.0", "8.6", "8.9", "9.0",
                ]
            } else {
                vec!["5.0", "5.2", "6.0", "6.1", "7.0", "7.5", "8.0", "8.6"]
            }
            .into_iter()
            .map(|s| s.to_string())
            .collect()
        } else {
            arches
        };

    // Map compute capability to readable names
    let readable: Vec<Object> = arch_list
        .iter()
        .map(|arch| {
            let name = match arch.as_str() {
                "3.0" | "3.5" | "3.7" => "Kepler",
                "5.0" | "5.2" | "5.3" => "Maxwell",
                "6.0" | "6.1" | "6.2" => "Pascal",
                "7.0" => "Volta",
                "7.5" => "Turing",
                "8.0" | "8.6" | "8.7" | "8.9" => "Ampere",
                "9.0" => "Hopper",
                _ => "Unknown",
            };
            Object::String(format!("SM{}({})", arch.replace('.', ""), name))
        })
        .collect();

    Ok(Object::Array(readable))
}
