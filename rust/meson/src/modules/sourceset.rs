/// Source set module: conditional source sets.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "sourceset.source_set".to_string()),
        sourceset_source_set,
    );
    // Methods on source_set objects
    vm.method_registry.insert(
        ("source_set".to_string(), "add".to_string()),
        source_set_add,
    );
    vm.method_registry.insert(
        ("source_set".to_string(), "add_all".to_string()),
        source_set_add_all,
    );
    vm.method_registry.insert(
        ("source_set".to_string(), "all_sources".to_string()),
        source_set_all_sources,
    );
    vm.method_registry.insert(
        ("source_set".to_string(), "all_dependencies".to_string()),
        source_set_all_dependencies,
    );
    vm.method_registry.insert(
        ("source_set".to_string(), "apply".to_string()),
        source_set_apply,
    );
    // Methods on source_set_result objects (returned by apply())
    vm.method_registry.insert(
        ("source_set_result".to_string(), "sources".to_string()),
        source_set_result_sources,
    );
    vm.method_registry.insert(
        ("source_set_result".to_string(), "dependencies".to_string()),
        source_set_result_dependencies,
    );
}

fn sourceset_source_set(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::SourceSet(SourceSetData::new()))
}

fn source_set_add(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::SourceSet(ss) = obj {
        let positional = VM::get_positional_args(args);

        let when = match VM::get_arg_value(args, "when") {
            Some(Object::Array(a)) => a.clone(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        };

        let if_true_kw = match VM::get_arg_value(args, "if_true") {
            Some(Object::Array(a)) => a.clone(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        };

        let if_false = match VM::get_arg_value(args, "if_false") {
            Some(Object::Array(a)) => a.clone(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        };

        // Positional args are added to if_true
        let mut if_true: Vec<Object> = positional.iter().map(|a| (*a).clone()).collect();
        if_true.extend(if_true_kw);

        ss.rules.borrow_mut().push(SourceSetRule {
            when,
            if_true,
            if_false,
        });

        Ok(obj.clone())
    } else {
        Err("source_set.add() called on non-source_set".to_string())
    }
}

fn source_set_add_all(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::SourceSet(ss) = obj {
        let positional = VM::get_positional_args(args);

        let when = match VM::get_arg_value(args, "when") {
            Some(Object::Array(a)) => a.clone(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        };

        let if_true = match VM::get_arg_value(args, "if_true") {
            Some(Object::Array(a)) => a.clone(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        };

        // add_all merges other source sets
        let mut rules = ss.rules.borrow_mut();
        for arg in &positional {
            if let Object::SourceSet(other) = *arg {
                rules.extend(other.rules.borrow().clone());
            }
        }

        if !when.is_empty() || !if_true.is_empty() {
            rules.push(SourceSetRule {
                when,
                if_true,
                if_false: Vec::new(),
            });
        }

        Ok(obj.clone())
    } else {
        Err("source_set.add_all() called on non-source_set".to_string())
    }
}

fn check_condition(when: &[Object], config: &Object) -> bool {
    for cond in when {
        match cond {
            Object::String(key) => {
                // Check if the config key is set and truthy
                if let Object::ConfigurationData(cfg) = config {
                    let values = cfg.values.borrow();
                    match values.get(key) {
                        Some((val, _)) => {
                            if !val.is_truthy() {
                                return false;
                            }
                        }
                        None => return false,
                    }
                } else if let Object::Dict(entries) = config {
                    match entries.iter().find(|(k, _)| k == key) {
                        Some((_, val)) => {
                            if !val.is_truthy() {
                                return false;
                            }
                        }
                        None => return false,
                    }
                }
            }
            Object::Dependency(d) => {
                if !d.found {
                    return false;
                }
            }
            other => {
                if !other.is_truthy() {
                    return false;
                }
            }
        }
    }
    true
}

fn collect_sources_deps(rules: &[SourceSetRule], config: &Object) -> (Vec<Object>, Vec<Object>) {
    let mut sources = Vec::new();
    let mut deps = Vec::new();

    for rule in rules {
        let matched = if rule.when.is_empty() {
            true
        } else {
            check_condition(&rule.when, config)
        };

        let items = if matched {
            &rule.if_true
        } else {
            &rule.if_false
        };

        for item in items {
            match item {
                Object::Dependency(_) => {
                    if !deps.contains(item) {
                        deps.push(item.clone());
                    }
                }
                _ => {
                    if !sources.contains(item) {
                        sources.push(item.clone());
                    }
                }
            }
        }

        // When conditions that are dependencies also get added to deps if matched
        if matched {
            for cond in &rule.when {
                if let Object::Dependency(d) = cond {
                    if d.found {
                        let dep_obj = Object::Dependency(d.clone());
                        if !deps.contains(&dep_obj) {
                            deps.push(dep_obj);
                        }
                    }
                }
            }
        }
    }

    (sources, deps)
}

fn source_set_apply(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::SourceSet(ss) = obj {
        let positional = VM::get_positional_args(args);
        let config = positional
            .first()
            .map(|v| (*v).clone())
            .unwrap_or(Object::None);

        let strict = VM::get_arg_bool(args, "strict", true);

        let rules = ss.rules.borrow();
        let (sources, deps) = collect_sources_deps(&rules, &config);

        // In strict mode, verify all when keys exist in config
        if strict {
            if let Object::ConfigurationData(cfg) = &config {
                let values = cfg.values.borrow();
                for rule in rules.iter() {
                    for cond in &rule.when {
                        if let Object::String(key) = cond {
                            if !values.contains_key(key) {
                                return Err(format!(
                                    "source_set.apply() strict mode: key '{}' not found in configuration",
                                    key
                                ));
                            }
                        }
                    }
                }
            }
        }

        Ok(Object::SourceSetResult(SourceSetResultData {
            sources,
            dependencies: deps,
        }))
    } else {
        Err("source_set.apply() called on non-source_set".to_string())
    }
}

fn source_set_all_sources(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::SourceSet(ss) = obj {
        let rules = ss.rules.borrow();
        let mut sources = Vec::new();
        for rule in rules.iter() {
            for item in &rule.if_true {
                if !matches!(item, Object::Dependency(_)) {
                    if !sources.contains(item) {
                        sources.push(item.clone());
                    }
                }
            }
            for item in &rule.if_false {
                if !matches!(item, Object::Dependency(_)) {
                    if !sources.contains(item) {
                        sources.push(item.clone());
                    }
                }
            }
        }
        Ok(Object::Array(sources))
    } else {
        Err("source_set.all_sources() called on non-source_set".to_string())
    }
}

fn source_set_all_dependencies(
    _vm: &mut VM,
    obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if let Object::SourceSet(ss) = obj {
        let rules = ss.rules.borrow();
        let mut deps = Vec::new();
        for rule in rules.iter() {
            for item in &rule.if_true {
                if matches!(item, Object::Dependency(_)) && !deps.contains(item) {
                    deps.push(item.clone());
                }
            }
            for cond in &rule.when {
                if matches!(cond, Object::Dependency(_)) && !deps.contains(cond) {
                    deps.push(cond.clone());
                }
            }
        }
        Ok(Object::Array(deps))
    } else {
        Err("source_set.all_dependencies() called on non-source_set".to_string())
    }
}

fn source_set_result_sources(
    _vm: &mut VM,
    obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if let Object::SourceSetResult(r) = obj {
        Ok(Object::Array(r.sources.clone()))
    } else {
        Err("Not a source set result".to_string())
    }
}

fn source_set_result_dependencies(
    _vm: &mut VM,
    obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if let Object::SourceSetResult(r) = obj {
        Ok(Object::Array(r.dependencies.clone()))
    } else {
        Err("Not a source set result".to_string())
    }
}
