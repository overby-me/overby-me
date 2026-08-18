/// External project module: add_project.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        (
            "module".to_string(),
            "external_project.add_project".to_string(),
        ),
        external_project_add_project,
    );
}

fn external_project_add_project(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let configure_command = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => {
            return Err(
                "external_project.add_project: first argument must be configure command"
                    .to_string(),
            );
        }
    };

    let configure_options = VM::get_arg_string_array(args, "configure_options");
    let _cross_configure_options = VM::get_arg_string_array(args, "cross_configure_options");
    let env = match VM::get_arg_value(args, "env") {
        Some(Object::Environment(e)) => e.to_map(),
        _ => std::collections::HashMap::new(),
    };
    let _verbose = VM::get_arg_bool(args, "verbose", false);
    let depends = match VM::get_arg_value(args, "depends") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let name = format!("external_{}", configure_command.replace('/', "_"));
    let _id = format!("ext_proj_{}", name);

    // Register as a run target that executes the external build
    vm.build_data.run_targets.push(RunTarget {
        name: name.clone(),
        command: {
            let mut cmd = vec![configure_command];
            cmd.extend(configure_options);
            cmd
        },
        depends,
        env,
        subdir: vm.current_subdir.clone(),
    });

    // Return a subproject-like object so .dependency() etc. can be called
    Ok(Object::Subproject(SubprojectData {
        name,
        version: String::new(),
        found: true,
        variables: std::collections::HashMap::new(),
    }))
}
