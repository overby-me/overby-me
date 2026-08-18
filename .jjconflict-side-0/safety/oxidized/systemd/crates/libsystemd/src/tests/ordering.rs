use crate::units::Unit;
use std::convert::TryInto;

#[test]
fn test_unit_ordering() {
    // Express the start chain with live [Unit] Requires= edges rather than
    // [Install] RequiredBy=. RequiredBy/WantedBy are enable-time metadata
    // (they only create .requires//.wants/ symlinks on `systemctl enable`)
    // and must not pull a unit into the start subgraph on their own; a live
    // pull is what Requires= expresses. The original chain was "2 requires
    // 1, 3 requires 2" written as reverse RequiredBy on 1 and 2; here it is
    // the equivalent forward Requires= on 2 and 3.
    let target1_str = format!(
        "
    [Unit]
    Description = {}
    Before = {}
    ",
        "Target", "2.target",
    );

    let parsed_file = crate::units::parse_file(&target1_str).unwrap();
    let target1_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/1.target"))
            .unwrap();

    let target2_str = format!(
        "
    [Unit]
    Description = {}
    After = {}
    Requires = {}
    ",
        "Target", "1.target", "1.target",
    );

    let parsed_file = crate::units::parse_file(&target2_str).unwrap();
    let target2_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/2.target"))
            .unwrap();

    let target3_str = format!(
        "
    [Unit]
    Description = {}
    After = {}
    After = {}
    Requires = {}
    ",
        "Target", "1.target", "2.target", "2.target"
    );

    let parsed_file = crate::units::parse_file(&target3_str).unwrap();
    let target3_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/3.target"))
            .unwrap();

    let mut unit_table = std::collections::HashMap::new();

    let target1_unit: Unit = target1_unit.try_into().unwrap();
    let target2_unit: Unit = target2_unit.try_into().unwrap();
    let target3_unit: Unit = target3_unit.try_into().unwrap();
    let id1 = target1_unit.id.clone();
    let id2 = target2_unit.id.clone();
    let id3 = target3_unit.id.clone();

    unit_table.insert(target1_unit.id.clone(), target1_unit);
    unit_table.insert(target2_unit.id.clone(), target2_unit);
    unit_table.insert(target3_unit.id.clone(), target3_unit);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());
    crate::units::sanity_check_dependencies(&unit_table).unwrap();

    unit_table
        .values()
        .for_each(|unit| println!("{} {:?}", unit.id, unit.common.dependencies));

    // before/after 1.target
    assert!(
        unit_table
            .get(&id1)
            .unwrap()
            .common
            .dependencies
            .after
            .is_empty()
    );
    assert!(
        unit_table
            .get(&id1)
            .unwrap()
            .common
            .dependencies
            .before
            .len()
            == 2
    );
    assert!(
        unit_table
            .get(&id1)
            .unwrap()
            .common
            .dependencies
            .before
            .contains(&id2)
    );
    assert!(
        unit_table
            .get(&id1)
            .unwrap()
            .common
            .dependencies
            .before
            .contains(&id3)
    );

    // before/after 2.target
    assert_eq!(
        unit_table
            .get(&id2)
            .unwrap()
            .common
            .dependencies
            .before
            .len(),
        1
    );
    assert!(
        unit_table
            .get(&id2)
            .unwrap()
            .common
            .dependencies
            .before
            .contains(&id3)
    );
    assert_eq!(
        unit_table
            .get(&id2)
            .unwrap()
            .common
            .dependencies
            .after
            .len(),
        1
    );
    assert!(
        unit_table
            .get(&id2)
            .unwrap()
            .common
            .dependencies
            .after
            .contains(&id1)
    );

    // before/after 3.target
    assert!(
        unit_table
            .get(&id3)
            .unwrap()
            .common
            .dependencies
            .before
            .is_empty()
    );
    assert!(
        unit_table
            .get(&id3)
            .unwrap()
            .common
            .dependencies
            .after
            .len()
            == 2
    );
    assert!(
        unit_table
            .get(&id3)
            .unwrap()
            .common
            .dependencies
            .after
            .contains(&id2)
    );
    assert!(
        unit_table
            .get(&id3)
            .unwrap()
            .common
            .dependencies
            .after
            .contains(&id1)
    );

    // Test the collection of start subgraphs
    // add a new unrelated unit, that should never occur in any of these operations for {1,2,3}.target
    let target4_str = format!(
        "
    [Unit]
    Description = {}

    ",
        "Target"
    );
    let parsed_file = crate::units::parse_file(&target4_str).unwrap();
    let target4_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/4.target"))
            .unwrap();
    let target4_unit: Unit = target4_unit.try_into().unwrap();
    let id4 = target4_unit.id.clone();
    unit_table.insert(target4_unit.id.clone(), target4_unit);

    // 3.target needs all units
    let mut ids_to_start = vec![id3.clone()];
    crate::units::collect_unit_start_subgraph(&mut ids_to_start, &unit_table);
    ids_to_start.sort();
    assert_eq!(ids_to_start, vec![id1.clone(), id2.clone(), id3.clone()]);

    // 2.target needs 1 and 2
    let mut ids_to_start = vec![id2.clone()];
    crate::units::collect_unit_start_subgraph(&mut ids_to_start, &unit_table);
    ids_to_start.sort();
    assert_eq!(ids_to_start, vec![id1.clone(), id2.clone()]);

    // 1.target needs only 1
    let mut ids_to_start = vec![id1.clone()];
    crate::units::collect_unit_start_subgraph(&mut ids_to_start, &unit_table);
    ids_to_start.sort();
    assert_eq!(ids_to_start, vec![id1.clone()]);

    // 4.target needs only 4
    let mut ids_to_start = vec![id4.clone()];
    crate::units::collect_unit_start_subgraph(&mut ids_to_start, &unit_table);
    ids_to_start.sort();
    assert_eq!(ids_to_start, vec![id4.clone()]);
}

#[test]
fn test_circle() {
    let target1_str = format!(
        "
    [Unit]
    Description = {}
    After = {}
    ",
        "Target", "3.target"
    );

    let parsed_file = crate::units::parse_file(&target1_str).unwrap();
    let target1_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/1.target"))
            .unwrap();

    let target2_str = format!(
        "
    [Unit]
    Description = {}
    After = {}
    ",
        "Target", "1.target"
    );

    let parsed_file = crate::units::parse_file(&target2_str).unwrap();
    let target2_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/2.target"))
            .unwrap();

    let target3_str = format!(
        "
    [Unit]
    Description = {}
    After = {}
    ",
        "Target", "2.target"
    );

    let parsed_file = crate::units::parse_file(&target3_str).unwrap();
    let target3_unit =
        crate::units::parse_target(parsed_file, &std::path::PathBuf::from("/path/to/3.target"))
            .unwrap();

    let mut unit_table = std::collections::HashMap::new();
    let target1_unit: Unit = target1_unit.try_into().unwrap();
    let target2_unit: Unit = target2_unit.try_into().unwrap();
    let target3_unit: Unit = target3_unit.try_into().unwrap();
    let target1_id = target1_unit.id.clone();
    let target2_id = target2_unit.id.clone();
    let target3_id = target3_unit.id.clone();
    unit_table.insert(target1_unit.id.clone(), target1_unit);
    unit_table.insert(target2_unit.id.clone(), target2_unit);
    unit_table.insert(target3_unit.id.clone(), target3_unit);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    if let Err(crate::units::SanityCheckError::CirclesFound(circles)) =
        crate::units::sanity_check_dependencies(&unit_table)
    {
        if circles.len() == 1 {
            let circle = &circles[0];
            assert_eq!(circle.len(), 3);
            assert!(circle.contains(&target1_id));
            assert!(circle.contains(&target2_id));
            assert!(circle.contains(&target3_id));
        } else {
            panic!("more than one circle found but there is only one");
        }
    } else {
        panic!("No circle found but there is one");
    }
}

/// Helper to create a minimal target unit string
fn target_unit_str(description: &str) -> String {
    format!(
        r#"
    [Unit]
    Description = {}
    "#,
        description
    )
}

/// Helper to create a target unit string with DefaultDependencies setting
fn target_unit_str_with_default_deps(description: &str, default_deps: &str) -> String {
    format!(
        r#"
    [Unit]
    Description = {}
    DefaultDependencies = {}
    "#,
        description, default_deps
    )
}

/// Helper to parse and convert a target unit
fn make_target(name: &str, content: &str) -> Unit {
    let parsed_file = crate::units::parse_file(content).unwrap();
    let target = crate::units::parse_target(
        parsed_file,
        &std::path::PathBuf::from(format!("/path/to/{}", name)),
    )
    .unwrap();
    target.try_into().unwrap()
}

/// Helper to parse and convert a service unit
fn make_service(name: &str, content: &str) -> Unit {
    let parsed_file = crate::units::parse_file(content).unwrap();
    let service = crate::units::parse_service(
        parsed_file,
        &std::path::PathBuf::from(format!("/path/to/{}", name)),
    )
    .unwrap();
    service.try_into().unwrap()
}

/// Helper to parse and convert a socket unit
fn make_socket(name: &str, content: &str) -> Unit {
    let parsed_file = crate::units::parse_file(content).unwrap();
    let socket = crate::units::parse_socket(
        parsed_file,
        &std::path::PathBuf::from(format!("/path/to/{}", name)),
    )
    .unwrap();
    socket.try_into().unwrap()
}

#[test]
fn test_default_deps_service_gets_shutdown_conflict_and_before() {
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let shutdown_id = shutdown.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let service = unit_table.get(&service_id).unwrap();
    assert!(
        service.common.dependencies.conflicts.contains(&shutdown_id),
        "Service should have Conflicts=shutdown.target"
    );
    assert!(
        service.common.dependencies.before.contains(&shutdown_id),
        "Service should have Before=shutdown.target"
    );
}

#[test]
fn test_default_deps_service_gets_sysinit_requires_and_after() {
    let sysinit = make_target("sysinit.target", &target_unit_str("System Initialization"));
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let sysinit_id = sysinit.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(sysinit.id.clone(), sysinit);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let service = unit_table.get(&service_id).unwrap();
    assert!(
        service.common.dependencies.requires.contains(&sysinit_id),
        "Service should have Requires=sysinit.target"
    );
    assert!(
        service.common.dependencies.after.contains(&sysinit_id),
        "Service should have After=sysinit.target"
    );
}

#[test]
fn test_default_deps_service_gets_after_basic_target() {
    let basic = make_target("basic.target", &target_unit_str("Basic System"));
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let basic_id = basic.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(basic.id.clone(), basic);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let service = unit_table.get(&service_id).unwrap();
    assert!(
        service.common.dependencies.after.contains(&basic_id),
        "Service should have After=basic.target"
    );
}

#[test]
fn test_default_deps_disabled_no_implicit_deps() {
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let sysinit = make_target("sysinit.target", &target_unit_str("System Initialization"));
    let basic = make_target("basic.target", &target_unit_str("Basic System"));
    let service = make_service(
        "myapp.service",
        r#"
        [Unit]
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let shutdown_id = shutdown.id.clone();
    let sysinit_id = sysinit.id.clone();
    let basic_id = basic.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(sysinit.id.clone(), sysinit);
    unit_table.insert(basic.id.clone(), basic);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let service = unit_table.get(&service_id).unwrap();
    assert!(
        !service.common.dependencies.conflicts.contains(&shutdown_id),
        "Service with DefaultDependencies=no should NOT have Conflicts=shutdown.target"
    );
    assert!(
        !service.common.dependencies.before.contains(&shutdown_id),
        "Service with DefaultDependencies=no should NOT have Before=shutdown.target"
    );
    assert!(
        !service.common.dependencies.requires.contains(&sysinit_id),
        "Service with DefaultDependencies=no should NOT have Requires=sysinit.target"
    );
    assert!(
        !service.common.dependencies.after.contains(&sysinit_id),
        "Service with DefaultDependencies=no should NOT have After=sysinit.target"
    );
    assert!(
        !service.common.dependencies.after.contains(&basic_id),
        "Service with DefaultDependencies=no should NOT have After=basic.target"
    );
}

#[test]
fn test_default_deps_target_only_gets_shutdown_not_sysinit() {
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let sysinit = make_target("sysinit.target", &target_unit_str("System Initialization"));
    let basic = make_target("basic.target", &target_unit_str("Basic System"));
    let custom_target = make_target("custom.target", &target_unit_str("Custom Target"));

    let shutdown_id = shutdown.id.clone();
    let sysinit_id = sysinit.id.clone();
    let basic_id = basic.id.clone();
    let custom_id = custom_target.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(sysinit.id.clone(), sysinit);
    unit_table.insert(basic.id.clone(), basic);
    unit_table.insert(custom_target.id.clone(), custom_target);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let custom = unit_table.get(&custom_id).unwrap();
    // Targets get Conflicts and Before on shutdown.target
    assert!(
        custom.common.dependencies.conflicts.contains(&shutdown_id),
        "Target should have Conflicts=shutdown.target"
    );
    assert!(
        custom.common.dependencies.before.contains(&shutdown_id),
        "Target should have Before=shutdown.target"
    );
    // Targets should NOT get Requires/After on sysinit.target or After on basic.target
    assert!(
        !custom.common.dependencies.requires.contains(&sysinit_id),
        "Target should NOT have Requires=sysinit.target"
    );
    assert!(
        !custom.common.dependencies.after.contains(&sysinit_id),
        "Target should NOT have After=sysinit.target"
    );
    assert!(
        !custom.common.dependencies.after.contains(&basic_id),
        "Target should NOT have After=basic.target"
    );
}

#[test]
fn test_default_deps_shutdown_target_excluded() {
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let sysinit = make_target("sysinit.target", &target_unit_str("System Initialization"));

    let shutdown_id = shutdown.id.clone();
    let sysinit_id = sysinit.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(sysinit.id.clone(), sysinit);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let shutdown = unit_table.get(&shutdown_id).unwrap();
    // shutdown.target should NOT have Conflicts/Before on itself
    assert!(
        !shutdown
            .common
            .dependencies
            .conflicts
            .contains(&shutdown_id),
        "shutdown.target should NOT conflict with itself"
    );
    assert!(
        !shutdown.common.dependencies.before.contains(&shutdown_id),
        "shutdown.target should NOT be Before itself"
    );
    // shutdown.target should NOT get Requires on sysinit (it's a target, not service)
    // but also it's excluded from default deps entirely
    assert!(
        !shutdown.common.dependencies.requires.contains(&sysinit_id),
        "shutdown.target should NOT have Requires=sysinit.target"
    );
}

#[test]
fn test_default_deps_reverse_relations_on_shutdown() {
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let shutdown_id = shutdown.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let shutdown = unit_table.get(&shutdown_id).unwrap();
    assert!(
        shutdown
            .common
            .dependencies
            .conflicted_by
            .contains(&service_id),
        "shutdown.target should have ConflictedBy=myapp.service (reverse of service's Conflicts)"
    );
    assert!(
        shutdown.common.dependencies.after.contains(&service_id),
        "shutdown.target should have After=myapp.service (reverse of service's Before)"
    );
}

#[test]
fn test_default_deps_reverse_relations_on_sysinit() {
    let sysinit = make_target("sysinit.target", &target_unit_str("System Initialization"));
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let sysinit_id = sysinit.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(sysinit.id.clone(), sysinit);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let sysinit = unit_table.get(&sysinit_id).unwrap();
    assert!(
        sysinit
            .common
            .dependencies
            .required_by
            .contains(&service_id),
        "sysinit.target should have RequiredBy=myapp.service (reverse of service's Requires)"
    );
    assert!(
        sysinit.common.dependencies.before.contains(&service_id),
        "sysinit.target should have Before=myapp.service (reverse of service's After)"
    );
}

#[test]
fn test_default_deps_reverse_relations_on_basic() {
    let basic = make_target("basic.target", &target_unit_str("Basic System"));
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let basic_id = basic.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(basic.id.clone(), basic);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let basic = unit_table.get(&basic_id).unwrap();
    assert!(
        basic.common.dependencies.before.contains(&service_id),
        "basic.target should have Before=myapp.service (reverse of service's After)"
    );
}

#[test]
fn test_default_deps_socket_gets_shutdown_and_sysinit_but_not_basic() {
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let sysinit = make_target("sysinit.target", &target_unit_str("System Initialization"));
    let basic = make_target("basic.target", &target_unit_str("Basic System"));
    let socket = make_socket(
        "myapp.socket",
        r#"
        [Socket]
        ListenStream = /run/myapp.sock
        "#,
    );

    let shutdown_id = shutdown.id.clone();
    let sysinit_id = sysinit.id.clone();
    let basic_id = basic.id.clone();
    let socket_id = socket.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(sysinit.id.clone(), sysinit);
    unit_table.insert(basic.id.clone(), basic);
    unit_table.insert(socket.id.clone(), socket);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let socket = unit_table.get(&socket_id).unwrap();
    // Sockets get shutdown conflict/before
    assert!(
        socket.common.dependencies.conflicts.contains(&shutdown_id),
        "Socket should have Conflicts=shutdown.target"
    );
    assert!(
        socket.common.dependencies.before.contains(&shutdown_id),
        "Socket should have Before=shutdown.target"
    );
    // Sockets get sysinit requires/after
    assert!(
        socket.common.dependencies.requires.contains(&sysinit_id),
        "Socket should have Requires=sysinit.target"
    );
    assert!(
        socket.common.dependencies.after.contains(&sysinit_id),
        "Socket should have After=sysinit.target"
    );
    // Sockets should NOT get basic.target (only services do)
    assert!(
        !socket.common.dependencies.after.contains(&basic_id),
        "Socket should NOT have After=basic.target"
    );
}

#[test]
fn test_default_deps_no_targets_present_is_noop() {
    // When none of shutdown/sysinit/basic targets exist, no implicit deps are added
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let service = unit_table.get(&service_id).unwrap();
    assert!(
        service.common.dependencies.conflicts.is_empty(),
        "Service should have no conflicts when no targets are present"
    );
    assert!(
        service.common.dependencies.requires.is_empty(),
        "Service should have no requires when no targets are present"
    );
    assert!(
        service.common.dependencies.before.is_empty(),
        "Service should have no before when no targets are present"
    );
    assert!(
        service.common.dependencies.after.is_empty(),
        "Service should have no after when no targets are present"
    );
}

#[test]
fn test_default_deps_mixed_enabled_and_disabled() {
    // One service with default deps, one without; only the one with should get implicit deps
    let shutdown = make_target("shutdown.target", &target_unit_str("Shutdown"));
    let service_with = make_service(
        "with.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );
    let service_without = make_service(
        "without.service",
        r#"
        [Unit]
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let shutdown_id = shutdown.id.clone();
    let with_id = service_with.id.clone();
    let without_id = service_without.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(service_with.id.clone(), service_with);
    unit_table.insert(service_without.id.clone(), service_without);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // Service with default deps should have shutdown conflict
    let with = unit_table.get(&with_id).unwrap();
    assert!(
        with.common.dependencies.conflicts.contains(&shutdown_id),
        "Service with DefaultDependencies=yes should have Conflicts=shutdown.target"
    );
    assert!(
        with.common.dependencies.before.contains(&shutdown_id),
        "Service with DefaultDependencies=yes should have Before=shutdown.target"
    );

    // Service without default deps should NOT have shutdown conflict
    let without = unit_table.get(&without_id).unwrap();
    assert!(
        !without.common.dependencies.conflicts.contains(&shutdown_id),
        "Service with DefaultDependencies=no should NOT have Conflicts=shutdown.target"
    );
    assert!(
        !without.common.dependencies.before.contains(&shutdown_id),
        "Service with DefaultDependencies=no should NOT have Before=shutdown.target"
    );

    // shutdown.target reverse relations should only reference the service with default deps
    let shutdown = unit_table.get(&shutdown_id).unwrap();
    assert!(
        shutdown
            .common
            .dependencies
            .conflicted_by
            .contains(&with_id),
        "shutdown.target should have ConflictedBy for service with default deps"
    );
    assert!(
        !shutdown
            .common
            .dependencies
            .conflicted_by
            .contains(&without_id),
        "shutdown.target should NOT have ConflictedBy for service without default deps"
    );
}

#[test]
fn test_default_deps_full_service_with_all_targets() {
    // Integration test: service with all three special targets present
    let shutdown = make_target(
        "shutdown.target",
        &target_unit_str_with_default_deps("Shutdown", "no"),
    );
    let sysinit = make_target(
        "sysinit.target",
        &target_unit_str_with_default_deps("System Initialization", "no"),
    );
    let basic = make_target(
        "basic.target",
        &target_unit_str_with_default_deps("Basic System", "no"),
    );
    let service = make_service(
        "myapp.service",
        r#"
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let shutdown_id = shutdown.id.clone();
    let sysinit_id = sysinit.id.clone();
    let basic_id = basic.id.clone();
    let service_id = service.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(shutdown.id.clone(), shutdown);
    unit_table.insert(sysinit.id.clone(), sysinit);
    unit_table.insert(basic.id.clone(), basic);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let service = unit_table.get(&service_id).unwrap();

    // shutdown.target relations
    assert!(service.common.dependencies.conflicts.contains(&shutdown_id));
    assert!(service.common.dependencies.before.contains(&shutdown_id));

    // sysinit.target relations
    assert!(service.common.dependencies.requires.contains(&sysinit_id));
    assert!(service.common.dependencies.after.contains(&sysinit_id));

    // basic.target relations
    assert!(service.common.dependencies.after.contains(&basic_id));

    // Verify reverse relations
    let shutdown = unit_table.get(&shutdown_id).unwrap();
    assert!(
        shutdown
            .common
            .dependencies
            .conflicted_by
            .contains(&service_id)
    );
    assert!(shutdown.common.dependencies.after.contains(&service_id));

    let sysinit = unit_table.get(&sysinit_id).unwrap();
    assert!(
        sysinit
            .common
            .dependencies
            .required_by
            .contains(&service_id)
    );
    assert!(sysinit.common.dependencies.before.contains(&service_id));

    let basic = unit_table.get(&basic_id).unwrap();
    assert!(basic.common.dependencies.before.contains(&service_id));
}

#[test]
fn test_conflicts_bidirectional() {
    // When unit A declares Conflicts=B, fill_dependencies should make it bidirectional:
    // A.conflicts contains B, B.conflicted_by contains A
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        &target_unit_str_with_default_deps("Target B", "no"),
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "A should have Conflicts=B"
    );

    let b = unit_table.get(&id_b).unwrap();
    assert!(
        b.common.dependencies.conflicted_by.contains(&id_a),
        "B should have ConflictedBy=A"
    );
}

#[test]
fn test_conflicts_one_way_does_not_reverse_conflicts_field() {
    // When only A declares Conflicts=B, the relationship is one-directional:
    // A.conflicts=[B], B.conflicted_by=[A]
    // But B does NOT get conflicts=[A], and A does NOT get conflicted_by=[B]
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        &target_unit_str_with_default_deps("Target B", "no"),
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // A declared the conflict, so A.conflicts has B
    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "A should have Conflicts=B"
    );

    // B gets the reverse: conflicted_by=[A]
    let b = unit_table.get(&id_b).unwrap();
    assert!(
        b.common.dependencies.conflicted_by.contains(&id_a),
        "B should have ConflictedBy=A"
    );

    // B should NOT get conflicts=[A] — only A declared the conflict
    assert!(
        !b.common.dependencies.conflicts.contains(&id_a),
        "B should NOT have Conflicts=A (only A declared the conflict)"
    );

    // A should NOT get conflicted_by=[B] — B didn't declare a conflict on A
    let a = unit_table.get(&id_a).unwrap();
    assert!(
        !a.common.dependencies.conflicted_by.contains(&id_b),
        "A should NOT have ConflictedBy=B (B did not declare a conflict)"
    );
}

#[test]
fn test_conflicts_mutual() {
    // Both A and B declare Conflicts on each other
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = Target B
        Conflicts = a.target
        DefaultDependencies = no
        "#,
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "A should have Conflicts=B"
    );
    assert!(
        a.common.dependencies.conflicted_by.contains(&id_b),
        "A should have ConflictedBy=B"
    );

    let b = unit_table.get(&id_b).unwrap();
    assert!(
        b.common.dependencies.conflicts.contains(&id_a),
        "B should have Conflicts=A"
    );
    assert!(
        b.common.dependencies.conflicted_by.contains(&id_a),
        "B should have ConflictedBy=A"
    );
}

#[test]
fn test_conflicts_dedup_after_fill() {
    // Mutual conflicts should be deduped so each ID appears only once
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = Target B
        Conflicts = a.target
        DefaultDependencies = no
        "#,
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    let count_b_in_conflicts = a
        .common
        .dependencies
        .conflicts
        .iter()
        .filter(|id| **id == id_b)
        .count();
    assert_eq!(
        count_b_in_conflicts, 1,
        "B should appear exactly once in A.conflicts after dedup"
    );

    let count_b_in_conflicted_by = a
        .common
        .dependencies
        .conflicted_by
        .iter()
        .filter(|id| **id == id_b)
        .count();
    assert_eq!(
        count_b_in_conflicted_by, 1,
        "B should appear exactly once in A.conflicted_by after dedup"
    );
}

#[test]
fn test_conflicts_multiple_targets() {
    // A conflicts with both B and C
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target,c.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        &target_unit_str_with_default_deps("Target B", "no"),
    );
    let target_c = make_target(
        "c.target",
        &target_unit_str_with_default_deps("Target C", "no"),
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();
    let id_c = target_c.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);
    unit_table.insert(target_c.id.clone(), target_c);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "A should have Conflicts=B"
    );
    assert!(
        a.common.dependencies.conflicts.contains(&id_c),
        "A should have Conflicts=C"
    );

    let b = unit_table.get(&id_b).unwrap();
    assert!(
        b.common.dependencies.conflicted_by.contains(&id_a),
        "B should have ConflictedBy=A"
    );

    let c = unit_table.get(&id_c).unwrap();
    assert!(
        c.common.dependencies.conflicted_by.contains(&id_a),
        "C should have ConflictedBy=A"
    );
}

#[test]
fn test_conflicts_service_with_service() {
    // Service-to-service conflict
    let svc_a = make_service(
        "a.service",
        r#"
        [Unit]
        Conflicts = b.service
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );
    let svc_b = make_service(
        "b.service",
        r#"
        [Unit]
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let id_a = svc_a.id.clone();
    let id_b = svc_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(svc_a.id.clone(), svc_a);
    unit_table.insert(svc_b.id.clone(), svc_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "Service A should have Conflicts=B"
    );

    let b = unit_table.get(&id_b).unwrap();
    assert!(
        b.common.dependencies.conflicted_by.contains(&id_a),
        "Service B should have ConflictedBy=A"
    );
}

#[test]
fn test_conflicts_cross_unit_types() {
    // Service conflicts with a target
    let service = make_service(
        "myapp.service",
        r#"
        [Unit]
        Conflicts = rescue.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );
    let target = make_target(
        "rescue.target",
        &target_unit_str_with_default_deps("Rescue", "no"),
    );

    let svc_id = service.id.clone();
    let tgt_id = target.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(service.id.clone(), service);
    unit_table.insert(target.id.clone(), target);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let svc = unit_table.get(&svc_id).unwrap();
    assert!(
        svc.common.dependencies.conflicts.contains(&tgt_id),
        "Service should have Conflicts=rescue.target"
    );

    let tgt = unit_table.get(&tgt_id).unwrap();
    assert!(
        tgt.common.dependencies.conflicted_by.contains(&svc_id),
        "rescue.target should have ConflictedBy=myapp.service"
    );
}

#[test]
fn test_conflicts_refs_by_name_includes_conflicts() {
    // Conflicts should be included in refs_by_name so pruning works correctly
    let service = make_service(
        "myapp.service",
        r#"
        [Unit]
        Conflicts = other.service
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let other_id: crate::units::UnitId = "other.service".try_into().unwrap();

    assert!(
        service.common.unit.refs_by_name.contains(&other_id),
        "refs_by_name should include conflicting unit IDs"
    );
}

#[test]
fn test_conflicts_no_conflict_when_not_specified() {
    // Two units with no conflict relationship
    let target_a = make_target(
        "a.target",
        &target_unit_str_with_default_deps("Target A", "no"),
    );
    let target_b = make_target(
        "b.target",
        &target_unit_str_with_default_deps("Target B", "no"),
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    assert!(
        !a.common.dependencies.conflicts.contains(&id_b),
        "A should NOT have Conflicts=B when not declared"
    );
    assert!(
        !a.common.dependencies.conflicted_by.contains(&id_b),
        "A should NOT have ConflictedBy=B when not declared"
    );

    let b = unit_table.get(&id_b).unwrap();
    assert!(
        !b.common.dependencies.conflicts.contains(&id_a),
        "B should NOT have Conflicts=A when not declared"
    );
    assert!(
        !b.common.dependencies.conflicted_by.contains(&id_a),
        "B should NOT have ConflictedBy=A when not declared"
    );
}

#[test]
fn test_conflicts_with_before_after_ordering() {
    // Conflicts can coexist with ordering relations
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target
        Before = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        &target_unit_str_with_default_deps("Target B", "no"),
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "A should have Conflicts=B"
    );
    assert!(
        a.common.dependencies.before.contains(&id_b),
        "A should have Before=B"
    );

    let b = unit_table.get(&id_b).unwrap();
    assert!(
        b.common.dependencies.conflicted_by.contains(&id_a),
        "B should have ConflictedBy=A"
    );
    assert!(
        b.common.dependencies.after.contains(&id_a),
        "B should have After=A (reverse of A's Before)"
    );
}

#[test]
fn test_conflicts_chain_three_units() {
    // A conflicts with B, B conflicts with C — conflicts are NOT transitive
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = Target A
        Conflicts = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = Target B
        Conflicts = c.target
        DefaultDependencies = no
        "#,
    );
    let target_c = make_target(
        "c.target",
        &target_unit_str_with_default_deps("Target C", "no"),
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();
    let id_c = target_c.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);
    unit_table.insert(target_c.id.clone(), target_c);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // A <-> B conflict
    let a = unit_table.get(&id_a).unwrap();
    assert!(a.common.dependencies.conflicts.contains(&id_b));

    // B <-> C conflict
    let b = unit_table.get(&id_b).unwrap();
    assert!(b.common.dependencies.conflicts.contains(&id_c));
    assert!(b.common.dependencies.conflicted_by.contains(&id_a));

    // A should NOT conflict with C (not transitive)
    let a = unit_table.get(&id_a).unwrap();
    assert!(
        !a.common.dependencies.conflicts.contains(&id_c),
        "Conflicts should NOT be transitive: A should not conflict with C"
    );
    assert!(
        !a.common.dependencies.conflicted_by.contains(&id_c),
        "Conflicts should NOT be transitive: A should not be conflicted_by C"
    );

    let c = unit_table.get(&id_c).unwrap();
    assert!(
        !c.common.dependencies.conflicts.contains(&id_a),
        "Conflicts should NOT be transitive: C should not conflict with A"
    );
    assert!(
        !c.common.dependencies.conflicted_by.contains(&id_a),
        "Conflicts should NOT be transitive: C should not be conflicted_by A"
    );
}

// ========================================================================
// Tests for break_dependency_cycles
// ========================================================================

#[test]
fn test_break_cycles_no_cycle() {
    // A DAG with no cycles: break_dependency_cycles should return empty and not modify anything
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        Before = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        Before = c.target
        DefaultDependencies = no
        "#,
    );
    let target_c = make_target("c.target", &target_unit_str_with_default_deps("C", "no"));

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();
    let id_c = target_c.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);
    unit_table.insert(target_c.id.clone(), target_c);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(broken.is_empty(), "No cycles should be found in a DAG");

    // Verify ordering is preserved
    let a = unit_table.get(&id_a).unwrap();
    assert!(a.common.dependencies.before.contains(&id_b));
    let b = unit_table.get(&id_b).unwrap();
    assert!(b.common.dependencies.before.contains(&id_c));
    assert!(b.common.dependencies.after.contains(&id_a));
    let c = unit_table.get(&id_c).unwrap();
    assert!(c.common.dependencies.after.contains(&id_b));
}

#[test]
fn test_break_cycles_simple_three_unit_cycle() {
    // 1 -> 2 -> 3 -> 1 (cycle via After edges, which produce Before reverse edges)
    let target1 = make_target(
        "1.target",
        r#"
        [Unit]
        Description = Target
        After = 3.target
        DefaultDependencies = no
        "#,
    );
    let target2 = make_target(
        "2.target",
        r#"
        [Unit]
        Description = Target
        After = 1.target
        DefaultDependencies = no
        "#,
    );
    let target3 = make_target(
        "3.target",
        r#"
        [Unit]
        Description = Target
        After = 2.target
        DefaultDependencies = no
        "#,
    );

    let id1 = target1.id.clone();
    let id2 = target2.id.clone();
    let id3 = target3.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target1.id.clone(), target1);
    unit_table.insert(target2.id.clone(), target2);
    unit_table.insert(target3.id.clone(), target3);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // Confirm cycle exists before breaking
    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Cycle should be detected before breaking"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(
        !broken.is_empty(),
        "Should have found and broken at least one cycle"
    );

    // After breaking, all three IDs should appear in the broken cycles
    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();
    assert!(all_ids_in_cycles.contains(&id1));
    assert!(all_ids_in_cycles.contains(&id2));
    assert!(all_ids_in_cycles.contains(&id3));

    // After breaking, the graph should be a valid DAG
    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking cycles"
    );
}

#[test]
fn test_break_cycles_two_unit_cycle() {
    // Minimal cycle: A before B, B before A
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        Before = b.target
        After = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        DefaultDependencies = no
        "#,
    );

    let id_a = target_a.id.clone();
    let id_b = target_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Should detect the 2-unit cycle"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty(), "Should break the 2-unit cycle");

    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();
    assert!(all_ids_in_cycles.contains(&id_a));
    assert!(all_ids_in_cycles.contains(&id_b));

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking the 2-unit cycle"
    );
}

#[test]
fn test_break_cycles_self_loop() {
    // A unit with Before and After pointing to itself
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        Before = a.target
        After = a.target
        DefaultDependencies = no
        "#,
    );

    let id_a = target_a.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);

    // Don't call fill_dependencies since self-referencing won't resolve to another unit,
    // but we set it up directly
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let _broken = crate::units::break_dependency_cycles(&mut unit_table);

    // After breaking, the self-loop edges should be removed
    let a = unit_table.get(&id_a).unwrap();
    assert!(
        !a.common.dependencies.before.contains(&id_a),
        "Self-loop before edge should be removed"
    );
    assert!(
        !a.common.dependencies.after.contains(&id_a),
        "Self-loop after edge should be removed"
    );

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking self-loop"
    );
}

#[test]
fn test_break_cycles_large_cycle() {
    // 5-unit cycle: 1 -> 2 -> 3 -> 4 -> 5 -> 1
    let t1 = make_target(
        "1.target",
        r#"
        [Unit]
        Description = 1
        After = 5.target
        DefaultDependencies = no
        "#,
    );
    let t2 = make_target(
        "2.target",
        r#"
        [Unit]
        Description = 2
        After = 1.target
        DefaultDependencies = no
        "#,
    );
    let t3 = make_target(
        "3.target",
        r#"
        [Unit]
        Description = 3
        After = 2.target
        DefaultDependencies = no
        "#,
    );
    let t4 = make_target(
        "4.target",
        r#"
        [Unit]
        Description = 4
        After = 3.target
        DefaultDependencies = no
        "#,
    );
    let t5 = make_target(
        "5.target",
        r#"
        [Unit]
        Description = 5
        After = 4.target
        DefaultDependencies = no
        "#,
    );

    let id1 = t1.id.clone();
    let id2 = t2.id.clone();
    let id3 = t3.id.clone();
    let id4 = t4.id.clone();
    let id5 = t5.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(t1.id.clone(), t1);
    unit_table.insert(t2.id.clone(), t2);
    unit_table.insert(t3.id.clone(), t3);
    unit_table.insert(t4.id.clone(), t4);
    unit_table.insert(t5.id.clone(), t5);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Should detect the 5-unit cycle"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty(), "Should break the 5-unit cycle");

    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();
    assert!(all_ids_in_cycles.contains(&id1));
    assert!(all_ids_in_cycles.contains(&id2));
    assert!(all_ids_in_cycles.contains(&id3));
    assert!(all_ids_in_cycles.contains(&id4));
    assert!(all_ids_in_cycles.contains(&id5));

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking the 5-unit cycle"
    );
}

#[test]
fn test_break_cycles_two_independent_cycles() {
    // Two disjoint cycles: {a,b,c} and {x,y,z}
    // Cycle 1: a -> b -> c -> a
    let a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        After = c.target
        DefaultDependencies = no
        "#,
    );
    let b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        After = a.target
        DefaultDependencies = no
        "#,
    );
    let c = make_target(
        "c.target",
        r#"
        [Unit]
        Description = C
        After = b.target
        DefaultDependencies = no
        "#,
    );
    // Cycle 2: x -> y -> z -> x
    let x = make_target(
        "x.target",
        r#"
        [Unit]
        Description = X
        After = z.target
        DefaultDependencies = no
        "#,
    );
    let y = make_target(
        "y.target",
        r#"
        [Unit]
        Description = Y
        After = x.target
        DefaultDependencies = no
        "#,
    );
    let z = make_target(
        "z.target",
        r#"
        [Unit]
        Description = Z
        After = y.target
        DefaultDependencies = no
        "#,
    );

    let id_a = a.id.clone();
    let id_b = b.id.clone();
    let id_c = c.id.clone();
    let id_x = x.id.clone();
    let id_y = y.id.clone();
    let id_z = z.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(a.id.clone(), a);
    unit_table.insert(b.id.clone(), b);
    unit_table.insert(c.id.clone(), c);
    unit_table.insert(x.id.clone(), x);
    unit_table.insert(y.id.clone(), y);
    unit_table.insert(z.id.clone(), z);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Should detect cycles before breaking"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty(), "Should break both cycles");

    // All six IDs should appear in the broken cycles
    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();
    assert!(all_ids_in_cycles.contains(&id_a));
    assert!(all_ids_in_cycles.contains(&id_b));
    assert!(all_ids_in_cycles.contains(&id_c));
    assert!(all_ids_in_cycles.contains(&id_x));
    assert!(all_ids_in_cycles.contains(&id_y));
    assert!(all_ids_in_cycles.contains(&id_z));

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking both cycles"
    );
}

#[test]
fn test_break_cycles_preserves_acyclic_ordering() {
    // Mix of cyclic and acyclic units:
    // Acyclic chain: p -> q (p before q)
    // Cycle: a -> b -> c -> a
    // The acyclic ordering must be preserved after breaking.
    let p = make_target(
        "p.target",
        r#"
        [Unit]
        Description = P
        Before = q.target
        DefaultDependencies = no
        "#,
    );
    let q = make_target("q.target", &target_unit_str_with_default_deps("Q", "no"));
    // Cycle
    let a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        After = c.target
        DefaultDependencies = no
        "#,
    );
    let b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        After = a.target
        DefaultDependencies = no
        "#,
    );
    let c = make_target(
        "c.target",
        r#"
        [Unit]
        Description = C
        After = b.target
        DefaultDependencies = no
        "#,
    );

    let id_p = p.id.clone();
    let id_q = q.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(p.id.clone(), p);
    unit_table.insert(q.id.clone(), q);
    unit_table.insert(a.id.clone(), a);
    unit_table.insert(b.id.clone(), b);
    unit_table.insert(c.id.clone(), c);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty(), "Should break the cycle");

    // Acyclic ordering p -> q must still be intact
    let p_unit = unit_table.get(&id_p).unwrap();
    assert!(
        p_unit.common.dependencies.before.contains(&id_q),
        "Acyclic Before edge p -> q should be preserved"
    );
    let q_unit = unit_table.get(&id_q).unwrap();
    assert!(
        q_unit.common.dependencies.after.contains(&id_p),
        "Acyclic After edge q -> p should be preserved"
    );

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking cycles"
    );
}

#[test]
fn test_break_cycles_preserves_non_ordering_deps() {
    // A cycle between two units that also have Wants/Requires/Conflicts.
    // Breaking the cycle should only remove Before/After edges, not other deps.
    let svc_a = make_service(
        "a.service",
        r#"
        [Unit]
        Description = A
        Wants = b.service
        Requires = b.service
        Conflicts = b.service
        Before = b.service
        After = b.service
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );
    let svc_b = make_service(
        "b.service",
        r#"
        [Unit]
        Description = B
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let id_a = svc_a.id.clone();
    let id_b = svc_b.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(svc_a.id.clone(), svc_a);
    unit_table.insert(svc_b.id.clone(), svc_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty(), "Should break the cycle");

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking"
    );

    // Non-ordering deps must be preserved
    let a = unit_table.get(&id_a).unwrap();
    assert!(
        a.common.dependencies.wants.contains(&id_b),
        "Wants should be preserved after cycle breaking"
    );
    assert!(
        a.common.dependencies.requires.contains(&id_b),
        "Requires should be preserved after cycle breaking"
    );
    assert!(
        a.common.dependencies.conflicts.contains(&id_b),
        "Conflicts should be preserved after cycle breaking"
    );
}

#[test]
fn test_break_cycles_removes_both_before_and_after_edges() {
    // Verify that when a cycle is broken, both the Before edge on the source unit
    // and the corresponding After edge on the destination unit are removed.
    let target_a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        After = b.target
        DefaultDependencies = no
        "#,
    );
    let target_b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        After = a.target
        DefaultDependencies = no
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target_a.id.clone(), target_a);
    unit_table.insert(target_b.id.clone(), target_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // Before breaking: a has before=[b], after=[b]; b has before=[a], after=[a]
    // (fill_dependencies makes the reverse edges)
    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty());

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking"
    );

    // One direction should remain and the other should be removed.
    // Verify consistency: for every Before edge X->Y, Y must have After edge Y->X, and vice versa.
    for unit in unit_table.values() {
        for before_id in &unit.common.dependencies.before {
            if let Some(target_unit) = unit_table.get(before_id) {
                assert!(
                    target_unit.common.dependencies.after.contains(&unit.id),
                    "Before edge {}->{} has no matching After edge",
                    unit.id,
                    before_id
                );
            }
        }
        for after_id in &unit.common.dependencies.after {
            if let Some(source_unit) = unit_table.get(after_id) {
                assert!(
                    source_unit.common.dependencies.before.contains(&unit.id),
                    "After edge {}->{} has no matching Before edge",
                    unit.id,
                    after_id
                );
            }
        }
    }
}

#[test]
fn test_break_cycles_returns_cycle_members() {
    // Verify that the returned Vec<Vec<UnitId>> contains exactly the units in each cycle
    let a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        After = c.target
        DefaultDependencies = no
        "#,
    );
    let b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        After = a.target
        DefaultDependencies = no
        "#,
    );
    let c = make_target(
        "c.target",
        r#"
        [Unit]
        Description = C
        After = b.target
        DefaultDependencies = no
        "#,
    );
    // d is not part of any cycle
    let d = make_target("d.target", &target_unit_str_with_default_deps("D", "no"));

    let id_a = a.id.clone();
    let id_b = b.id.clone();
    let id_c = c.id.clone();
    let id_d = d.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(a.id.clone(), a);
    unit_table.insert(b.id.clone(), b);
    unit_table.insert(c.id.clone(), c);
    unit_table.insert(d.id.clone(), d);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    let broken = crate::units::break_dependency_cycles(&mut unit_table);

    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();

    // The cycle members should be present
    assert!(all_ids_in_cycles.contains(&id_a));
    assert!(all_ids_in_cycles.contains(&id_b));
    assert!(all_ids_in_cycles.contains(&id_c));

    // The non-cycle member should NOT appear in any broken cycle
    assert!(
        !all_ids_in_cycles.contains(&id_d),
        "Non-cyclic unit d should not appear in broken cycles"
    );
}

#[test]
fn test_break_cycles_service_target_mixed_cycle() {
    // Cycle across different unit types: service -> target -> service
    let svc = make_service(
        "app.service",
        r#"
        [Unit]
        Description = App
        Before = setup.target
        After = setup.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );
    let tgt = make_target(
        "setup.target",
        &target_unit_str_with_default_deps("Setup", "no"),
    );

    let svc_id = svc.id.clone();
    let tgt_id = tgt.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(svc.id.clone(), svc);
    unit_table.insert(tgt.id.clone(), tgt);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Should detect the cross-type cycle"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(!broken.is_empty(), "Should break the cross-type cycle");

    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();
    assert!(all_ids_in_cycles.contains(&svc_id));
    assert!(all_ids_in_cycles.contains(&tgt_id));

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking cross-type cycle"
    );
}

#[test]
fn test_break_cycles_finds_real_cycle_not_fake_dump() {
    // Regression test: previously, when no node had in-degree 0, the cycle
    // detection would dump ALL remaining nodes from a HashSet as a single
    // "cycle" in arbitrary order. The cycle-breaker would then try to remove
    // an edge between two arbitrary nodes that might not even be connected,
    // causing an infinite loop (the "spam" bug).
    //
    // This test sets up a chain with a real cycle embedded in it:
    //   a -> b -> c -> d -> e -> f -> c  (cycle is c -> d -> e -> f -> c)
    // The fix should find the actual cycle path, not dump all 6 nodes.
    let a = make_target(
        "a.target",
        r#"
        [Unit]
        Description = A
        Before = b.target
        DefaultDependencies = no
        "#,
    );
    let b = make_target(
        "b.target",
        r#"
        [Unit]
        Description = B
        Before = c.target
        After = a.target
        DefaultDependencies = no
        "#,
    );
    let c = make_target(
        "c.target",
        r#"
        [Unit]
        Description = C
        Before = d.target
        After = b.target
        After = f.target
        DefaultDependencies = no
        "#,
    );
    let d = make_target(
        "d.target",
        r#"
        [Unit]
        Description = D
        Before = e.target
        After = c.target
        DefaultDependencies = no
        "#,
    );
    let e = make_target(
        "e.target",
        r#"
        [Unit]
        Description = E
        Before = f.target
        After = d.target
        DefaultDependencies = no
        "#,
    );
    let f = make_target(
        "f.target",
        r#"
        [Unit]
        Description = F
        Before = c.target
        After = e.target
        DefaultDependencies = no
        "#,
    );

    let id_a = a.id.clone();
    let id_b = b.id.clone();
    let id_c = c.id.clone();
    let id_d = d.id.clone();
    let id_e = e.id.clone();
    let id_f = f.id.clone();

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(a.id.clone(), a);
    unit_table.insert(b.id.clone(), b);
    unit_table.insert(c.id.clone(), c);
    unit_table.insert(d.id.clone(), d);
    unit_table.insert(e.id.clone(), e);
    unit_table.insert(f.id.clone(), f);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // Confirm cycle exists
    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Cycle should be detected"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(
        !broken.is_empty(),
        "Should have found and broken at least one cycle"
    );

    // The cycle members should be a subset of {c, d, e, f} — a and b are not in any cycle
    let all_ids_in_cycles: std::collections::HashSet<_> =
        broken.iter().flatten().cloned().collect();
    assert!(
        !all_ids_in_cycles.contains(&id_a),
        "a.target is not part of any cycle"
    );
    assert!(
        !all_ids_in_cycles.contains(&id_b),
        "b.target is not part of any cycle"
    );
    // At least some of the real cycle members should be present
    let cycle_members_found = [&id_c, &id_d, &id_e, &id_f]
        .iter()
        .filter(|id| all_ids_in_cycles.contains(id))
        .count();
    assert!(
        cycle_members_found >= 2,
        "At least 2 of the real cycle members should appear in broken cycles, found {}",
        cycle_members_found
    );

    // After breaking, the graph should be a valid DAG
    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking cycles"
    );
}

#[test]
fn test_cycle_detection_ignores_nonexistent_after_deps() {
    // Regression test: if a unit has After= referencing a unit that doesn't
    // exist in the table, the in-degree calculation should ignore it.
    // Previously, non-existent deps were counted, making in-degree > 0 for
    // all nodes, which triggered the fallback that dumped everything as a
    // "cycle" and caused an infinite loop.
    let svc = make_service(
        "myapp.service",
        r#"
        [Unit]
        Description = My App
        After = nonexistent.service
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );
    let tgt = make_target(
        "default.target",
        r#"
        [Unit]
        Description = Default
        DefaultDependencies = no
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(svc.id.clone(), svc);
    unit_table.insert(tgt.id.clone(), tgt);

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    // There is no cycle — the non-existent After= dep should be ignored
    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Non-existent After= dependency should not cause a false cycle"
    );

    // break_dependency_cycles should also complete without hanging
    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(
        broken.is_empty(),
        "No cycles should be found when After= references a non-existent unit"
    );
}

#[test]
fn test_cycle_detection_many_nodes_one_cycle() {
    // Stress test: 10-node chain with a cycle at the end.
    // Ensures the DFS-based cycle finder scales and doesn't
    // exhibit O(n!) behavior or infinite loops.
    //
    // Chain: n1 -> n2 -> n3 -> ... -> n8 -> n9 -> n10 -> n8 (cycle: n8 -> n9 -> n10 -> n8)
    let names: Vec<String> = (1..=10).map(|i| format!("n{}.target", i)).collect();

    let mut units = Vec::new();
    for i in 0..10 {
        let mut before_str = String::new();
        let mut after_str = String::new();

        // Chain: each node is Before the next
        if i < 9 {
            before_str = format!("Before = {}", names[i + 1]);
        }
        if i > 0 {
            after_str = format!("After = {}", names[i - 1]);
        }
        // Close the cycle: n10 (index 9) is Before n8 (index 7)
        if i == 9 {
            before_str = format!("Before = {}", names[7]);
        }
        // n8 (index 7) also has After = n10
        if i == 7 {
            after_str = format!("{}\nAfter = {}", after_str, names[9]);
        }

        let unit_str = format!(
            r#"
            [Unit]
            Description = Node {}
            {}
            {}
            DefaultDependencies = no
            "#,
            i + 1,
            before_str,
            after_str
        );

        units.push(make_target(&names[i], &unit_str));
    }

    let mut unit_table = std::collections::HashMap::new();
    for u in units {
        unit_table.insert(u.id.clone(), u);
    }

    crate::units::fill_dependencies(&mut unit_table).unwrap();
    unit_table
        .values_mut()
        .for_each(|unit| unit.dedup_dependencies());

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_err(),
        "Cycle should be detected in the 10-node graph"
    );

    let broken = crate::units::break_dependency_cycles(&mut unit_table);
    assert!(
        !broken.is_empty(),
        "Should have broken at least one cycle in the 10-node graph"
    );

    assert!(
        crate::units::sanity_check_dependencies(&unit_table).is_ok(),
        "Graph should be a DAG after breaking cycles in the 10-node graph"
    );
}

// ============================================================
// PartOf= dependency resolution tests
// ============================================================

#[test]
fn test_part_of_fills_part_of_by() {
    // When unit A has PartOf=B, then B should get part_of_by=[A]
    let target = make_target(
        "network.target",
        r#"
        [Unit]
        Description = Network
        DefaultDependencies = no
        "#,
    );

    let service = make_service(
        "helper.service",
        r#"
        [Unit]
        Description = Helper
        PartOf = network.target
        DefaultDependencies = no
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target.id.clone(), target);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();

    let target_id = crate::units::UnitId {
        name: "network.target".to_owned(),
        kind: crate::units::UnitIdKind::Target,
    };
    let service_id = crate::units::UnitId {
        name: "helper.service".to_owned(),
        kind: crate::units::UnitIdKind::Service,
    };

    // The target should have part_of_by pointing to the service
    let target_unit = unit_table.get(&target_id).unwrap();
    assert!(
        target_unit
            .common
            .dependencies
            .part_of_by
            .contains(&service_id),
        "network.target should have helper.service in part_of_by"
    );

    // The service should have part_of pointing to the target
    let service_unit = unit_table.get(&service_id).unwrap();
    assert!(
        service_unit
            .common
            .dependencies
            .part_of
            .contains(&target_id),
        "helper.service should have network.target in part_of"
    );
}

#[test]
fn test_part_of_multiple_fills_part_of_by() {
    // When units A and B both have PartOf=C, then C should get part_of_by=[A, B]
    let target = make_target(
        "app.target",
        r#"
        [Unit]
        Description = App
        DefaultDependencies = no
        "#,
    );

    let svc_a = make_service(
        "worker-a.service",
        r#"
        [Unit]
        PartOf = app.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let svc_b = make_service(
        "worker-b.service",
        r#"
        [Unit]
        PartOf = app.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target.id.clone(), target);
    unit_table.insert(svc_a.id.clone(), svc_a);
    unit_table.insert(svc_b.id.clone(), svc_b);

    crate::units::fill_dependencies(&mut unit_table).unwrap();

    let target_id = crate::units::UnitId {
        name: "app.target".to_owned(),
        kind: crate::units::UnitIdKind::Target,
    };
    let svc_a_id = crate::units::UnitId {
        name: "worker-a.service".to_owned(),
        kind: crate::units::UnitIdKind::Service,
    };
    let svc_b_id = crate::units::UnitId {
        name: "worker-b.service".to_owned(),
        kind: crate::units::UnitIdKind::Service,
    };

    let target_unit = unit_table.get(&target_id).unwrap();
    assert!(
        target_unit
            .common
            .dependencies
            .part_of_by
            .contains(&svc_a_id),
        "app.target should have worker-a.service in part_of_by"
    );
    assert!(
        target_unit
            .common
            .dependencies
            .part_of_by
            .contains(&svc_b_id),
        "app.target should have worker-b.service in part_of_by"
    );
}

#[test]
fn test_part_of_kill_before_this_includes_part_of_by() {
    // kill_before_this() should include part_of_by units so they are stopped
    // when the target unit is stopped
    let target = make_target(
        "network.target",
        r#"
        [Unit]
        Description = Network
        DefaultDependencies = no
        "#,
    );

    let service = make_service(
        "net-helper.service",
        r#"
        [Unit]
        PartOf = network.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target.id.clone(), target);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();

    let target_id = crate::units::UnitId {
        name: "network.target".to_owned(),
        kind: crate::units::UnitIdKind::Target,
    };
    let service_id = crate::units::UnitId {
        name: "net-helper.service".to_owned(),
        kind: crate::units::UnitIdKind::Service,
    };

    let target_unit = unit_table.get(&target_id).unwrap();
    let kill_list = target_unit.common.dependencies.kill_before_this();
    assert!(
        kill_list.contains(&service_id),
        "kill_before_this() should include PartOf dependents: {:?}",
        kill_list
    );
}

#[test]
fn test_part_of_refs_by_name_includes_part_of() {
    // PartOf targets should be included in refs_by_name so pruning works correctly
    let service = make_service(
        "helper.service",
        r#"
        [Unit]
        PartOf = network.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let target_id = crate::units::UnitId {
        name: "network.target".to_owned(),
        kind: crate::units::UnitIdKind::Target,
    };

    assert!(
        service.common.unit.refs_by_name.contains(&target_id),
        "refs_by_name should include PartOf unit IDs"
    );
}

#[test]
fn test_part_of_no_part_of_by_when_not_specified() {
    // Units without PartOf= should have empty part_of_by after fill_dependencies
    let target = make_target(
        "basic.target",
        r#"
        [Unit]
        Description = Basic
        DefaultDependencies = no
        "#,
    );

    let service = make_service(
        "simple.service",
        r#"
        [Unit]
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target.id.clone(), target);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();

    let target_id = crate::units::UnitId {
        name: "basic.target".to_owned(),
        kind: crate::units::UnitIdKind::Target,
    };

    let target_unit = unit_table.get(&target_id).unwrap();
    assert!(
        target_unit.common.dependencies.part_of_by.is_empty(),
        "part_of_by should be empty when no unit declares PartOf= this unit"
    );
}

#[test]
fn test_part_of_combined_with_requires() {
    // PartOf= and Requires= can both point to the same unit.
    // Both relationships should be tracked independently.
    let target = make_target(
        "app.target",
        r#"
        [Unit]
        Description = App
        DefaultDependencies = no
        "#,
    );

    let service = make_service(
        "component.service",
        r#"
        [Unit]
        PartOf = app.target
        Requires = app.target
        DefaultDependencies = no
        [Service]
        ExecStart = /bin/true
        "#,
    );

    let mut unit_table = std::collections::HashMap::new();
    unit_table.insert(target.id.clone(), target);
    unit_table.insert(service.id.clone(), service);

    crate::units::fill_dependencies(&mut unit_table).unwrap();

    let target_id = crate::units::UnitId {
        name: "app.target".to_owned(),
        kind: crate::units::UnitIdKind::Target,
    };
    let service_id = crate::units::UnitId {
        name: "component.service".to_owned(),
        kind: crate::units::UnitIdKind::Service,
    };

    let target_unit = unit_table.get(&target_id).unwrap();
    assert!(
        target_unit
            .common
            .dependencies
            .part_of_by
            .contains(&service_id),
        "app.target should have component.service in part_of_by"
    );
    assert!(
        target_unit
            .common
            .dependencies
            .required_by
            .contains(&service_id),
        "app.target should have component.service in required_by"
    );

    // kill_before_this should include the service from both relationships
    let kill_list = target_unit.common.dependencies.kill_before_this();
    assert!(
        kill_list.contains(&service_id),
        "kill_before_this() should include the service"
    );
}
