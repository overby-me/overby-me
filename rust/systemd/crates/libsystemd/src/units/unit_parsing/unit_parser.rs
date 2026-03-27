//! Parse all supported unit types / options for these and do needed operations like matching services <-> sockets and adding implicit dependencies like
//! all sockets to socket.target

use log::{debug, trace};

use crate::units::{
    EnvVars, ParsedExecSection, ParsedInstallSection, ParsedUnitSection, ParsingErrorReason,
    RLimitValue, ResourceLimit, StdIoOption, UnitAction,
};
use std::collections::HashMap;
use std::path::PathBuf;

pub type ParsedSection = HashMap<String, Vec<(u32, String)>>;
pub type ParsedFile = HashMap<String, ParsedSection>;

pub fn parse_file(content: &str) -> Result<ParsedFile, ParsingErrorReason> {
    let mut sections = HashMap::new();

    // Handle line continuation: lines ending with '\' are joined with the
    // next line (the backslash and newline are removed, and leading
    // whitespace on the continuation line is collapsed to a single space).
    // This matches systemd's INI parser behavior per systemd.syntax(7).
    let raw_lines: Vec<&str> = content.split('\n').collect();
    let mut joined_lines: Vec<String> = Vec::with_capacity(raw_lines.len());
    let mut accumulator = String::new();
    for raw_line in &raw_lines {
        let trimmed = raw_line.trim();
        if let Some(prefix) = trimmed.strip_suffix('\\') {
            // Continuation: append content before the backslash
            if !accumulator.is_empty() {
                accumulator.push(' ');
            }
            accumulator.push_str(prefix.trim_end());
        } else {
            // Final line (no trailing backslash)
            if !accumulator.is_empty() {
                accumulator.push(' ');
                accumulator.push_str(trimmed);
                joined_lines.push(std::mem::take(&mut accumulator));
            } else {
                joined_lines.push(trimmed.to_string());
            }
        }
    }
    // Flush any remaining accumulator (file ending with backslash)
    if !accumulator.is_empty() {
        joined_lines.push(accumulator);
    }

    let lines: Vec<&str> = joined_lines.iter().map(|s| s.as_str()).collect();
    let mut lines_left = &lines[..];

    // remove lines before the first section
    while !lines_left.is_empty() && !lines_left[0].starts_with('[') {
        lines_left = &lines_left[1..];
    }
    if lines_left.is_empty() {
        return Ok(sections);
    }
    let mut current_section_name: String = lines_left[0].into();
    let mut current_section_lines = Vec::new();

    lines_left = &lines_left[1..];

    while !lines_left.is_empty() {
        let line = lines_left[0];

        if line.starts_with('[') {
            // Merge with existing section if it already appeared (systemd
            // merges duplicate section headers).
            let parsed = parse_section(&current_section_lines);
            sections
                .entry(current_section_name.clone())
                .and_modify(|existing: &mut ParsedSection| {
                    for (key, values) in &parsed {
                        existing
                            .entry(key.clone())
                            .or_default()
                            .extend(values.iter().cloned());
                    }
                })
                .or_insert(parsed);
            current_section_name = line.into();
            current_section_lines.clear();
        } else {
            current_section_lines.push(line);
        }
        lines_left = &lines_left[1..];
    }

    // insert last section (merge if it already exists)
    let parsed = parse_section(&current_section_lines);
    sections
        .entry(current_section_name)
        .and_modify(|existing: &mut ParsedSection| {
            for (key, values) in &parsed {
                existing
                    .entry(key.clone())
                    .or_default()
                    .extend(values.iter().cloned());
            }
        })
        .or_insert(parsed);

    Ok(sections)
}

#[must_use]
pub fn map_tuples_to_second<X, Y: Clone>(v: Vec<(X, Y)>) -> Vec<Y> {
    v.iter().map(|(_, scnd)| scnd.clone()).collect()
}

/// Split comma-separated and space-separated values in list-type fields into
/// individual entries. In systemd, dependency fields like After=, Before=,
/// Wants=, Requires=, Conflicts= accept both comma-separated and
/// space-separated values. This function handles both kinds of splitting for
/// list-type fields. It must NOT be used for command-line fields (ExecStart=,
/// etc.) where commas can be part of arguments.
pub(crate) fn split_list_values(tuples: Vec<(u32, String)>) -> Vec<(u32, String)> {
    let mut result = Vec::new();
    for (idx, value) in tuples {
        for part in value.split(|c: char| c == ',' || c.is_whitespace()) {
            let part = part.trim();
            if !part.is_empty() {
                result.push((idx, part.to_string()));
            }
        }
    }
    result
}

#[must_use]
pub fn string_to_bool(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let s_upper = &s.to_uppercase();
    let c: char = s_upper.chars().next().unwrap();

    let is_num_and_one = s.len() == 1 && c == '1';
    *s_upper == *"YES" || *s_upper == *"TRUE" || is_num_and_one
}

fn parse_environment(raw_line: &str) -> Result<EnvVars, ParsingErrorReason> {
    debug!("raw line: {raw_line}");
    let split = shlex::split(raw_line).ok_or(ParsingErrorReason::Generic(format!(
        "Could not parse cmdline: {raw_line}"
    )))?;
    debug!("split: {split:?}");
    let mut vars: Vec<(String, String)> = Vec::new();

    for pair in split {
        let p: Vec<&str> = pair.splitn(2, '=').collect();
        let key = p[0].to_owned();
        let val = if p.len() > 1 { p[1] } else { "" };
        vars.push((key, val.to_owned()));
    }

    Ok(EnvVars { vars })
}

/// Convert an absolute path to a systemd-style mount unit name.
///
/// Examples:
/// - `/` → `-.mount`
/// - `/var` → `var.mount`
/// - `/var/log` → `var-log.mount`
#[allow(dead_code)]
pub(crate) fn path_to_mount_unit_name(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "-.mount".to_owned()
    } else {
        format!("{}.mount", trimmed.replace('/', "-"))
    }
}

/// Convert a device path to the corresponding `.device` unit name.
///
/// This follows the same escaping convention as mount units:
/// - `/dev/sda1` → `dev-sda1.device`
/// - `/dev/disk/by-uuid/1234` → `dev-disk-by\x2duuid-1234.device`
/// - `/sys/devices/pci0000:00/net/eth0` → `sys-devices-pci0000:00-net-eth0.device`
///
/// For simple `/dev/` and `/sys/` paths, the slash-to-dash replacement
/// is sufficient. For paths containing characters outside `[a-zA-Z0-9:_.]`,
/// full unit name escaping via `unit_name_path_escape` should be used instead.
#[allow(dead_code)]
pub(crate) fn path_to_device_unit_name(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "-.device".to_owned()
    } else {
        format!("{}.device", trimmed.replace('/', "-"))
    }
}

/// Return mount unit names for every prefix of the given absolute path,
/// from `/` down to the path itself.
///
/// For `/var/log/myapp` this returns:
/// `["-.mount", "var.mount", "var-log.mount", "var-log-myapp.mount"]`
pub(crate) fn mount_units_for_path(path: &str) -> Vec<String> {
    let mut units = vec!["-.mount".to_owned()];
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return units;
    }
    let mut accumulated = String::new();
    for component in trimmed.split('/') {
        if !accumulated.is_empty() {
            accumulated.push('-');
        }
        accumulated.push_str(component);
        let name = format!("{accumulated}.mount");
        if !units.contains(&name) {
            units.push(name);
        }
    }
    units
}

fn parse_unit_action(value: &str) -> Result<UnitAction, ParsingErrorReason> {
    match value.to_lowercase().replace('-', "").as_str() {
        "none" => Ok(UnitAction::None),
        "exit" => Ok(UnitAction::Exit),
        "exitforce" => Ok(UnitAction::ExitForce),
        "reboot" => Ok(UnitAction::Reboot),
        "rebootforce" => Ok(UnitAction::RebootForce),
        "rebootimmediate" => Ok(UnitAction::RebootImmediate),
        "poweroff" => Ok(UnitAction::Poweroff),
        "poweroffforce" => Ok(UnitAction::PoweroffForce),
        "poweroffimmediate" => Ok(UnitAction::PoweroffImmediate),
        "halt" => Ok(UnitAction::Halt),
        "haltforce" => Ok(UnitAction::HaltForce),
        "haltimmediate" => Ok(UnitAction::HaltImmediate),
        "kexec" => Ok(UnitAction::Kexec),
        "kexecforce" => Ok(UnitAction::KexecForce),
        "kexecimmediate" => Ok(UnitAction::KexecImmediate),
        other => Err(ParsingErrorReason::UnknownSetting(
            "SuccessAction/FailureAction/JobTimeoutAction".to_owned(),
            other.to_owned(),
        )),
    }
}

pub fn parse_unit_section(
    mut section: ParsedSection,
) -> Result<ParsedUnitSection, ParsingErrorReason> {
    let wants = section.remove("WANTS");
    let requires = section.remove("REQUIRES");
    let conflicts = section.remove("CONFLICTS");
    let after = section.remove("AFTER");
    let before = section.remove("BEFORE");
    let description = section.remove("DESCRIPTION");
    let documentation = section.remove("DOCUMENTATION");
    let default_dependencies = section.remove("DEFAULTDEPENDENCIES");
    let condition_path_exists = section.remove("CONDITIONPATHEXISTS");
    let condition_path_is_directory = section.remove("CONDITIONPATHISDIRECTORY");
    let condition_virtualization = section.remove("CONDITIONVIRTUALIZATION");
    let condition_capability = section.remove("CONDITIONCAPABILITY");
    let condition_first_boot = section.remove("CONDITIONFIRSTBOOT");
    let condition_file_is_executable = section.remove("CONDITIONFILEISEXECUTABLE");
    let condition_file_not_empty = section.remove("CONDITIONFILENOTEMPTY");
    let condition_kernel_module_loaded = section.remove("CONDITIONKERNELMODULELOADED");
    let condition_directory_not_empty = section.remove("CONDITIONDIRECTORYNOTEMPTY");
    let condition_kernel_command_line = section.remove("CONDITIONKERNELCOMMANDLINE");
    let condition_control_group_controller = section.remove("CONDITIONCONTROLGROUPCONTROLLER");
    let condition_path_is_read_write = section.remove("CONDITIONPATHISREADWRITE");
    let condition_needs_update = section.remove("CONDITIONNEEDSUPDATE");
    let condition_path_is_mount_point = section.remove("CONDITIONPATHISMOUNTPOINT");
    let condition_security = section.remove("CONDITIONSECURITY");
    let condition_ac_power = section.remove("CONDITIONACPOWER");
    let condition_architecture = section.remove("CONDITIONARCHITECTURE");
    let condition_environment = section.remove("CONDITIONENVIRONMENT");
    let condition_firmware = section.remove("CONDITIONFIRMWARE");
    let condition_host = section.remove("CONDITIONHOST");
    let condition_memory = section.remove("CONDITIONMEMORY");
    let condition_cpu_feature = section.remove("CONDITIONCPUFEATURE");
    let condition_cpus = section.remove("CONDITIONCPUS");
    let condition_os_release = section.remove("CONDITIONOSRELEASE");
    let condition_path_is_encrypted = section.remove("CONDITIONPATHISENCRYPTED");
    let condition_path_is_symbolic_link = section.remove("CONDITIONPATHISSYMBOLICLINK");
    let condition_user = section.remove("CONDITIONUSER");
    let condition_group = section.remove("CONDITIONGROUP");

    // Assert* directives — same semantics as Condition* but cause the unit
    // to FAIL (not silently skip) when the check is false.
    let assert_path_exists = section.remove("ASSERTPATHEXISTS");
    let assert_path_is_directory = section.remove("ASSERTPATHISDIRECTORY");
    let assert_virtualization = section.remove("ASSERTVIRTUALIZATION");
    let assert_capability = section.remove("ASSERTCAPABILITY");
    let assert_first_boot = section.remove("ASSERTFIRSTBOOT");
    let assert_file_is_executable = section.remove("ASSERTFILEISEXECUTABLE");
    let assert_file_not_empty = section.remove("ASSERTFILENOTEMPTY");
    let assert_kernel_module_loaded = section.remove("ASSERTKERNELMODULELOADED");
    let assert_directory_not_empty = section.remove("ASSERTDIRECTORYNOTEMPTY");
    let assert_kernel_command_line = section.remove("ASSERTKERNELCOMMANDLINE");
    let assert_control_group_controller = section.remove("ASSERTCONTROLGROUPCONTROLLER");
    let assert_path_is_read_write = section.remove("ASSERTPATHISREADWRITE");
    let assert_needs_update = section.remove("ASSERTNEEDSUPDATE");
    let assert_path_is_mount_point = section.remove("ASSERTPATHISMOUNTPOINT");
    let assert_security = section.remove("ASSERTSECURITY");
    let assert_ac_power = section.remove("ASSERTACPOWER");
    let assert_architecture = section.remove("ASSERTARCHITECTURE");
    let assert_environment = section.remove("ASSERTENVIRONMENT");
    let assert_firmware = section.remove("ASSERTFIRMWARE");
    let assert_host = section.remove("ASSERTHOST");
    let assert_memory = section.remove("ASSERTMEMORY");
    let assert_cpu_feature = section.remove("ASSERTCPUFEATURE");
    let assert_cpus = section.remove("ASSERTCPUS");
    let assert_os_release = section.remove("ASSERTOSRELEASE");
    let assert_path_is_encrypted = section.remove("ASSERTPATHISENCRYPTED");
    let assert_path_is_symbolic_link = section.remove("ASSERTPATHISSYMBOLICLINK");
    let assert_user = section.remove("ASSERTUSER");
    let assert_group = section.remove("ASSERTGROUP");

    let success_action = section.remove("SUCCESSACTION");
    let failure_action = section.remove("FAILUREACTION");
    let part_of = section.remove("PARTOF");
    let binds_to = section.remove("BINDSTO");
    let ignore_on_isolate = section.remove("IGNOREONISOLATE");
    let requires_mounts_for = section.remove("REQUIRESMOUNTSFOR");
    let stop_when_unneeded = section.remove("STOPWHENUNNEEDED");
    let allow_isolate = section.remove("ALLOWISOLATE");
    let job_timeout_sec = section.remove("JOBTIMEOUTSEC");
    let job_timeout_action = section.remove("JOBTIMEOUTACTION");
    let refuse_manual_start = section.remove("REFUSEMANUALSTART");
    let refuse_manual_stop = section.remove("REFUSEMANUALSTOP");
    let requisite = section.remove("REQUISITE");
    let upholds = section.remove("UPHOLDS");
    let on_success = section.remove("ONSUCCESS");
    let on_failure = section.remove("ONFAILURE");
    let on_failure_job_mode = section.remove("ONFAILUREJOBMODE");
    let on_success_job_mode = section.remove("ONSUCCESSJOBMODE");
    let propagates_reload_to = section.remove("PROPAGATESRELOADTO");
    let reload_propagated_from = section.remove("RELOADPROPAGATEDFROM");
    let propagates_stop_to = section.remove("PROPAGATESSTOPTO");
    let stop_propagated_from = section.remove("STOPPROPAGATEDFROM");
    // StopPropagatedFrom= is the reverse of PropagatesStopTo= — merge into propagates_stop_to
    // for now since both express the same relationship from different perspectives.
    let _ = stop_propagated_from; // consumed but not yet used for reverse dep injection
    let joins_namespace_of = section.remove("JOINSNAMESPACEOF");
    let start_limit_interval_sec = section.remove("STARTLIMITINTERVALSEC");
    let start_limit_burst = section.remove("STARTLIMITBURST");
    let start_limit_action = section.remove("STARTLIMITACTION");
    let success_action_exit_status = section.remove("SUCCESSACTIONEXITSTATUS");
    let failure_action_exit_status = section.remove("FAILUREACTIONEXITSTATUS");
    let job_running_timeout_sec = section.remove("JOBRUNNINGTIMEOUTSEC");
    let job_timeout_reboot_argument = section.remove("JOBTIMEOUTREBOOTARGUMENT");
    let collect_mode = section.remove("COLLECTMODE");
    let source_path = section.remove("SOURCEPATH");
    let reboot_argument = section.remove("REBOOTARGUMENT");

    for key in section.keys() {
        if key.starts_with("X-") {
            trace!("Silently ignoring vendor extension in [Unit] section: {key}");
            continue;
        }
        trace!("Ignoring unsupported setting in [Unit] section: {key}");
    }

    // Parse RequiresMountsFor= paths and generate implicit mount unit deps
    let requires_mounts_for_paths: Vec<String> =
        map_tuples_to_second(split_list_values(requires_mounts_for.unwrap_or_default()));

    let mut mount_unit_requires: Vec<String> = Vec::new();
    let mut mount_unit_after: Vec<String> = Vec::new();
    for path in &requires_mounts_for_paths {
        for unit_name in mount_units_for_path(path) {
            if !mount_unit_requires.contains(&unit_name) {
                mount_unit_requires.push(unit_name.clone());
            }
            if !mount_unit_after.contains(&unit_name) {
                mount_unit_after.push(unit_name);
            }
        }
    }

    let default_dependencies = default_dependencies
        .map(|x| string_to_bool(&x[0].1))
        .unwrap_or(true);

    let ignore_on_isolate = ignore_on_isolate
        .map(|x| string_to_bool(&x[0].1))
        .unwrap_or(false);

    let stop_when_unneeded = stop_when_unneeded
        .map(|x| string_to_bool(&x[0].1))
        .unwrap_or(false);

    let allow_isolate = allow_isolate
        .map(|x| string_to_bool(&x[0].1))
        .unwrap_or(false);

    let refuse_manual_start = refuse_manual_start
        .map(|x| string_to_bool(&x[0].1))
        .unwrap_or(false);

    let refuse_manual_stop = refuse_manual_stop
        .map(|x| string_to_bool(&x[0].1))
        .unwrap_or(false);

    let requisite_list = map_tuples_to_second(split_list_values(requisite.unwrap_or_default()));
    let upholds_list = map_tuples_to_second(split_list_values(upholds.unwrap_or_default()));
    let on_success_list = map_tuples_to_second(split_list_values(on_success.unwrap_or_default()));
    let on_failure_list = map_tuples_to_second(split_list_values(on_failure.unwrap_or_default()));
    let propagates_reload_to_list =
        map_tuples_to_second(split_list_values(propagates_reload_to.unwrap_or_default()));
    let reload_propagated_from_list = map_tuples_to_second(split_list_values(
        reload_propagated_from.unwrap_or_default(),
    ));
    let propagates_stop_to_list =
        map_tuples_to_second(split_list_values(propagates_stop_to.unwrap_or_default()));
    let joins_namespace_of_list =
        map_tuples_to_second(split_list_values(joins_namespace_of.unwrap_or_default()));

    let on_success_job_mode = match on_success_job_mode {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.trim().to_lowercase().as_str() {
                    "replace" | "" => super::OnFailureJobMode::Replace,
                    "fail" => super::OnFailureJobMode::Fail,
                    "replace-irreversibly" => super::OnFailureJobMode::ReplaceIrreversibly,
                    "isolate" => super::OnFailureJobMode::Isolate,
                    "flush" => super::OnFailureJobMode::Flush,
                    "ignore-dependencies" => super::OnFailureJobMode::IgnoreDependencies,
                    "ignore-requirements" => super::OnFailureJobMode::IgnoreRequirements,
                    other => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "OnSuccessJobMode".to_owned(),
                            other.to_owned(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "OnSuccessJobMode".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => super::OnFailureJobMode::default(),
    };

    let on_failure_job_mode = match on_failure_job_mode {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.trim().to_lowercase().as_str() {
                    "replace" | "" => super::OnFailureJobMode::Replace,
                    "fail" => super::OnFailureJobMode::Fail,
                    "replace-irreversibly" => super::OnFailureJobMode::ReplaceIrreversibly,
                    "isolate" => super::OnFailureJobMode::Isolate,
                    "flush" => super::OnFailureJobMode::Flush,
                    "ignore-dependencies" => super::OnFailureJobMode::IgnoreDependencies,
                    "ignore-requirements" => super::OnFailureJobMode::IgnoreRequirements,
                    other => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "OnFailureJobMode".to_owned(),
                            other.to_owned(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "OnFailureJobMode".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => super::OnFailureJobMode::default(),
    };

    let conditions = parse_condition_or_assert_entries(
        condition_path_exists,
        condition_path_is_directory,
        condition_virtualization,
        condition_capability,
        condition_first_boot,
        condition_file_is_executable,
        condition_file_not_empty,
        condition_kernel_module_loaded,
        condition_directory_not_empty,
        condition_kernel_command_line,
        condition_control_group_controller,
        condition_path_is_read_write,
        condition_needs_update,
        condition_path_is_mount_point,
        condition_security,
        condition_ac_power,
        condition_architecture,
        condition_environment,
        condition_firmware,
        condition_host,
        condition_memory,
        condition_cpu_feature,
        condition_cpus,
        condition_os_release,
        condition_path_is_encrypted,
        condition_path_is_symbolic_link,
        condition_user,
        condition_group,
    );

    let assertions = parse_condition_or_assert_entries(
        assert_path_exists,
        assert_path_is_directory,
        assert_virtualization,
        assert_capability,
        assert_first_boot,
        assert_file_is_executable,
        assert_file_not_empty,
        assert_kernel_module_loaded,
        assert_directory_not_empty,
        assert_kernel_command_line,
        assert_control_group_controller,
        assert_path_is_read_write,
        assert_needs_update,
        assert_path_is_mount_point,
        assert_security,
        assert_ac_power,
        assert_architecture,
        assert_environment,
        assert_firmware,
        assert_host,
        assert_memory,
        assert_cpu_feature,
        assert_cpus,
        assert_os_release,
        assert_path_is_encrypted,
        assert_path_is_symbolic_link,
        assert_user,
        assert_group,
    );

    let success_action = match success_action {
        Some(vec) => {
            if vec.len() == 1 {
                parse_unit_action(&vec[0].1)?
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "SuccessAction".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => UnitAction::default(),
    };

    let failure_action = match failure_action {
        Some(vec) => {
            if vec.len() == 1 {
                parse_unit_action(&vec[0].1)?
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "FailureAction".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => UnitAction::default(),
    };

    let job_timeout_sec = match job_timeout_sec {
        Some(vec) => {
            if vec.len() == 1 {
                Some(super::service_unit::parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "JobTimeoutSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let job_timeout_action = match job_timeout_action {
        Some(vec) => {
            if vec.len() == 1 {
                parse_unit_action(&vec[0].1)?
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "JobTimeoutAction".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => UnitAction::default(),
    };

    let start_limit_interval_sec = match start_limit_interval_sec {
        Some(vec) => {
            if vec.len() == 1 {
                Some(super::service_unit::parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "StartLimitIntervalSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let start_limit_burst = match start_limit_burst {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.trim().parse::<u32>() {
                    Ok(val) => Some(val),
                    Err(_) => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "StartLimitBurst".to_owned(),
                            vec[0].1.clone(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "StartLimitBurst".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let start_limit_action = match start_limit_action {
        Some(vec) => {
            if vec.len() == 1 {
                parse_unit_action(&vec[0].1)?
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "StartLimitAction".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => UnitAction::default(),
    };

    let success_action_exit_status = match success_action_exit_status {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.trim().parse::<u8>() {
                    Ok(val) => Some(val),
                    Err(_) => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "SuccessActionExitStatus".to_owned(),
                            vec[0].1.clone(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "SuccessActionExitStatus".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let failure_action_exit_status = match failure_action_exit_status {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.trim().parse::<u8>() {
                    Ok(val) => Some(val),
                    Err(_) => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "FailureActionExitStatus".to_owned(),
                            vec[0].1.clone(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "FailureActionExitStatus".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let job_running_timeout_sec = match job_running_timeout_sec {
        Some(vec) => {
            if vec.len() == 1 {
                Some(super::service_unit::parse_timeout(&vec[0].1))
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "JobRunningTimeoutSec".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => None,
    };

    let job_timeout_reboot_argument = job_timeout_reboot_argument.map(|x| x[0].1.clone());

    let collect_mode = match collect_mode {
        Some(vec) => {
            if vec.len() == 1 {
                match vec[0].1.trim().to_lowercase().as_str() {
                    "inactive" | "" => super::CollectMode::Inactive,
                    "inactive-or-failed" => super::CollectMode::InactiveOrFailed,
                    other => {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "CollectMode".to_owned(),
                            other.to_owned(),
                        ));
                    }
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "CollectMode".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => super::CollectMode::default(),
    };

    let source_path = source_path.map(|x| x[0].1.clone());
    let reboot_argument = reboot_argument.map(|x| x[0].1.clone());

    // Merge explicit deps with implicit mount deps from RequiresMountsFor=
    let mut requires_list = map_tuples_to_second(split_list_values(requires.unwrap_or_default()));
    // Add Requisite= deps to requires (they also need After= semantics at runtime,
    // but for ordering purposes they go into the requires list here)
    for name in &requisite_list {
        if !requires_list.contains(name) {
            requires_list.push(name.clone());
        }
    }
    for name in mount_unit_requires {
        if !requires_list.contains(&name) {
            requires_list.push(name);
        }
    }

    let mut after_list = map_tuples_to_second(split_list_values(after.unwrap_or_default()));
    for name in mount_unit_after {
        if !after_list.contains(&name) {
            after_list.push(name);
        }
    }

    Ok(ParsedUnitSection {
        description: description.map(|x| (x[0]).1.clone()).unwrap_or_default(),
        documentation: map_tuples_to_second(split_list_values(documentation.unwrap_or_default())),
        wants: map_tuples_to_second(split_list_values(wants.unwrap_or_default())),
        requires: requires_list,
        requisite: requisite_list,
        upholds: upholds_list,
        conflicts: map_tuples_to_second(split_list_values(conflicts.unwrap_or_default())),
        after: after_list,
        before: map_tuples_to_second(split_list_values(before.unwrap_or_default())),
        part_of: map_tuples_to_second(split_list_values(part_of.unwrap_or_default())),
        binds_to: map_tuples_to_second(split_list_values(binds_to.unwrap_or_default())),
        default_dependencies,
        ignore_on_isolate,
        conditions,
        assertions,
        success_action,
        failure_action,
        success_action_exit_status,
        failure_action_exit_status,
        requires_mounts_for: requires_mounts_for_paths,
        stop_when_unneeded,
        allow_isolate,
        job_timeout_sec,
        job_timeout_action,
        job_running_timeout_sec,
        job_timeout_reboot_argument,
        refuse_manual_start,
        refuse_manual_stop,
        on_success: on_success_list,
        on_success_job_mode,
        on_failure: on_failure_list,
        on_failure_job_mode,
        propagates_reload_to: propagates_reload_to_list,
        reload_propagated_from: reload_propagated_from_list,
        propagates_stop_to: propagates_stop_to_list,
        joins_namespace_of: joins_namespace_of_list,
        start_limit_interval_sec,
        start_limit_burst,
        start_limit_action,
        collect_mode,
        source_path,
        reboot_argument,
    })
}

/// Shared helper for parsing both Condition* and Assert* entries into
/// `Vec<UnitCondition>`.  The two directive families use identical syntax
/// and semantics — only the failure mode differs (skip vs. fail), which is
/// handled by the caller.
#[allow(clippy::too_many_arguments)]
fn parse_condition_or_assert_entries(
    path_exists: Option<Vec<(u32, String)>>,
    path_is_directory: Option<Vec<(u32, String)>>,
    virtualization: Option<Vec<(u32, String)>>,
    capability: Option<Vec<(u32, String)>>,
    first_boot: Option<Vec<(u32, String)>>,
    file_is_executable: Option<Vec<(u32, String)>>,
    file_not_empty: Option<Vec<(u32, String)>>,
    kernel_module_loaded: Option<Vec<(u32, String)>>,
    directory_not_empty: Option<Vec<(u32, String)>>,
    kernel_command_line: Option<Vec<(u32, String)>>,
    control_group_controller: Option<Vec<(u32, String)>>,
    path_is_read_write: Option<Vec<(u32, String)>>,
    needs_update: Option<Vec<(u32, String)>>,
    path_is_mount_point: Option<Vec<(u32, String)>>,
    security: Option<Vec<(u32, String)>>,
    ac_power: Option<Vec<(u32, String)>>,
    architecture: Option<Vec<(u32, String)>>,
    environment: Option<Vec<(u32, String)>>,
    firmware: Option<Vec<(u32, String)>>,
    host: Option<Vec<(u32, String)>>,
    memory: Option<Vec<(u32, String)>>,
    cpu_feature: Option<Vec<(u32, String)>>,
    cpus: Option<Vec<(u32, String)>>,
    os_release: Option<Vec<(u32, String)>>,
    path_is_encrypted: Option<Vec<(u32, String)>>,
    path_is_symbolic_link: Option<Vec<(u32, String)>>,
    user: Option<Vec<(u32, String)>>,
    group: Option<Vec<(u32, String)>>,
) -> Vec<super::UnitCondition> {
    let mut out = Vec::new();

    for (_, value) in path_exists.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::PathExists { path, negate });
    }
    for (_, value) in path_is_directory.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::PathIsDirectory { path, negate });
    }
    for (_, raw) in virtualization.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.to_lowercase(), true)
        } else {
            (trimmed.to_lowercase(), false)
        };
        if !value.is_empty() {
            out.push(super::UnitCondition::Virtualization { value, negate });
        }
    }
    for (_, raw) in capability.unwrap_or_default() {
        let trimmed = raw.trim();
        let (cap, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !cap.is_empty() {
            out.push(super::UnitCondition::Capability {
                capability: cap,
                negate,
            });
        }
    }
    for (_, raw) in first_boot.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value_str, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim(), true)
        } else {
            (trimmed, false)
        };
        let value = string_to_bool(value_str);
        out.push(super::UnitCondition::FirstBoot { value, negate });
    }
    for (_, value) in file_is_executable.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::FileIsExecutable { path, negate });
    }
    for (_, value) in kernel_module_loaded.unwrap_or_default() {
        let (module, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::KernelModuleLoaded { module, negate });
    }
    for (_, value) in file_not_empty.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::FileNotEmpty { path, negate });
    }
    for (_, value) in directory_not_empty.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::DirectoryNotEmpty { path, negate });
    }
    for (_, raw) in kernel_command_line.unwrap_or_default() {
        let trimmed = raw.trim();
        let (argument, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !argument.is_empty() {
            out.push(super::UnitCondition::KernelCommandLine { argument, negate });
        }
    }
    for (_, value) in path_is_read_write.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::PathIsReadWrite { path, negate });
    }
    for (_, raw) in control_group_controller.unwrap_or_default() {
        let trimmed = raw.trim();
        let (controller, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !controller.is_empty() {
            out.push(super::UnitCondition::ControlGroupController { controller, negate });
        }
    }
    for (_, value) in needs_update.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::NeedsUpdate { path, negate });
    }
    for (_, value) in path_is_mount_point.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::PathIsMountPoint { path, negate });
    }
    for (_, raw) in security.unwrap_or_default() {
        let trimmed = raw.trim();
        let (technology, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !technology.is_empty() {
            out.push(super::UnitCondition::Security { technology, negate });
        }
    }
    for (_, raw) in ac_power.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value_str, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim(), true)
        } else {
            (trimmed, false)
        };
        let value = string_to_bool(value_str);
        out.push(super::UnitCondition::ACPower { value, negate });
    }
    for (_, raw) in architecture.unwrap_or_default() {
        let trimmed = raw.trim();
        let (arch, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_lowercase(), true)
        } else {
            (trimmed.to_lowercase(), false)
        };
        if !arch.is_empty() {
            out.push(super::UnitCondition::Architecture { arch, negate });
        }
    }
    for (_, raw) in environment.unwrap_or_default() {
        let trimmed = raw.trim();
        let (expression, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !expression.is_empty() {
            out.push(super::UnitCondition::Environment { expression, negate });
        }
    }
    for (_, raw) in firmware.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !value.is_empty() {
            out.push(super::UnitCondition::Firmware { value, negate });
        }
    }
    for (_, raw) in host.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !value.is_empty() {
            out.push(super::UnitCondition::Host { value, negate });
        }
    }
    for (_, raw) in memory.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !value.is_empty() {
            out.push(super::UnitCondition::Memory { value, negate });
        }
    }
    for (_, raw) in cpu_feature.unwrap_or_default() {
        let trimmed = raw.trim();
        let (feature, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !feature.is_empty() {
            out.push(super::UnitCondition::CPUFeature { feature, negate });
        }
    }
    for (_, raw) in cpus.unwrap_or_default() {
        let trimmed = raw.trim();
        let (expression, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !expression.is_empty() {
            out.push(super::UnitCondition::CPUs { expression, negate });
        }
    }
    for (_, raw) in os_release.unwrap_or_default() {
        let trimmed = raw.trim();
        let (expression, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !expression.is_empty() {
            out.push(super::UnitCondition::OSRelease { expression, negate });
        }
    }
    for (_, value) in path_is_encrypted.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::PathIsEncrypted { path, negate });
    }
    for (_, value) in path_is_symbolic_link.unwrap_or_default() {
        let (path, negate) = if let Some(stripped) = value.strip_prefix('!') {
            (stripped.to_string(), true)
        } else {
            (value, false)
        };
        out.push(super::UnitCondition::PathIsSymbolicLink { path, negate });
    }
    for (_, raw) in user.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !value.is_empty() {
            out.push(super::UnitCondition::User { value, negate });
        }
    }
    for (_, raw) in group.unwrap_or_default() {
        let trimmed = raw.trim();
        let (value, negate) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (stripped.trim().to_owned(), true)
        } else {
            (trimmed.to_owned(), false)
        };
        if !value.is_empty() {
            out.push(super::UnitCondition::Group { value, negate });
        }
    }

    out
}

fn make_stdio_option(setting: &str) -> Result<StdIoOption, ParsingErrorReason> {
    match setting.to_lowercase().as_str() {
        "null" | "" => Ok(StdIoOption::Null),
        "inherit" => Ok(StdIoOption::Inherit),
        "tty" => Ok(StdIoOption::Tty),
        "journal" | "syslog" | "journal+console" | "syslog+console" => Ok(StdIoOption::Journal),
        "kmsg" | "kmsg+console" => Ok(StdIoOption::Kmsg),
        _ if setting.starts_with("file:") => {
            let p = setting.trim_start_matches("file:");
            Ok(StdIoOption::File(p.into()))
        }
        _ if setting.starts_with("append:") => {
            let p = setting.trim_start_matches("append:");
            Ok(StdIoOption::AppendFile(p.into()))
        }
        _ if setting.starts_with("truncate:") => {
            let p = setting.trim_start_matches("truncate:");
            Ok(StdIoOption::File(p.into()))
        }
        _ => {
            trace!(
                "Unsupported StandardOutput/StandardError={}, treating as inherit",
                setting
            );
            Ok(StdIoOption::Inherit)
        }
    }
}

/// Parse a resource limit value in the format used by all LimitXXX= directives.
/// Accepts a single value, a "soft:hard" pair, or "infinity". Byte-suffix values
/// (K, M, G, T, P, E) are supported for byte-oriented limits when `byte_limit`
/// is true. Returns None if the input vector is None or empty-string (reset).
fn parse_resource_limit(
    name: &str,
    vec: Option<Vec<(u32, String)>>,
    byte_limit: bool,
) -> Result<Option<ResourceLimit>, ParsingErrorReason> {
    let vec = match vec {
        Some(v) => v,
        None => return Ok(None),
    };
    if vec.len() != 1 {
        return Err(ParsingErrorReason::SettingTooManyValues(
            name.to_owned(),
            super::map_tuples_to_second(vec),
        ));
    }
    let val = vec[0].1.trim();
    if val.is_empty() {
        return Ok(None);
    }
    if val.eq_ignore_ascii_case("infinity") {
        return Ok(Some(ResourceLimit {
            soft: RLimitValue::Infinity,
            hard: RLimitValue::Infinity,
        }));
    }
    if let Some((soft_str, hard_str)) = val.split_once(':') {
        let soft = parse_rlimit_value(name, soft_str.trim(), byte_limit)?;
        let hard = parse_rlimit_value(name, hard_str.trim(), byte_limit)?;
        Ok(Some(ResourceLimit { soft, hard }))
    } else {
        let v = parse_rlimit_value(name, val, byte_limit)?;
        Ok(Some(ResourceLimit { soft: v, hard: v }))
    }
}

/// Parse a single rlimit value: a number (with optional byte suffix if
/// `byte_limit` is true) or "infinity".
fn parse_rlimit_value(
    name: &str,
    s: &str,
    byte_limit: bool,
) -> Result<RLimitValue, ParsingErrorReason> {
    if s.eq_ignore_ascii_case("infinity") {
        return Ok(RLimitValue::Infinity);
    }
    if byte_limit {
        // Try to parse with optional K/M/G/T/P/E suffix
        let (num_str, multiplier) =
            if let Some(prefix) = s.strip_suffix('E').or_else(|| s.strip_suffix('e')) {
                (prefix, 1024u64 * 1024 * 1024 * 1024 * 1024 * 1024)
            } else if let Some(prefix) = s.strip_suffix('P').or_else(|| s.strip_suffix('p')) {
                (prefix, 1024u64 * 1024 * 1024 * 1024 * 1024)
            } else if let Some(prefix) = s.strip_suffix('T').or_else(|| s.strip_suffix('t')) {
                (prefix, 1024u64 * 1024 * 1024 * 1024)
            } else if let Some(prefix) = s.strip_suffix('G').or_else(|| s.strip_suffix('g')) {
                (prefix, 1024u64 * 1024 * 1024)
            } else if let Some(prefix) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
                (prefix, 1024u64 * 1024)
            } else if let Some(prefix) = s.strip_suffix('K').or_else(|| s.strip_suffix('k')) {
                (prefix, 1024u64)
            } else {
                (s, 1u64)
            };
        let num: u64 = num_str
            .parse()
            .map_err(|_| ParsingErrorReason::Generic(format!("{name} value is not valid: {s}")))?;
        Ok(RLimitValue::Value(num.saturating_mul(multiplier)))
    } else {
        let num: u64 = s
            .parse()
            .map_err(|_| ParsingErrorReason::Generic(format!("{name} value is not valid: {s}")))?;
        Ok(RLimitValue::Value(num))
    }
}

/// Parse a space-separated list that accumulates across multiple directives.
/// An empty assignment resets the list.
fn parse_space_separated_list(vec: Option<Vec<(u32, String)>>) -> Vec<String> {
    match vec {
        Some(vec) => {
            let mut entries = Vec::new();
            for (_idx, line) in &vec {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    entries.clear();
                    continue;
                }
                for token in trimmed.split_whitespace() {
                    entries.push(token.to_owned());
                }
            }
            entries
        }
        None => Vec::new(),
    }
}

/// Parse an octal mode value (e.g. "0755") used by directory mode directives.
fn parse_octal_mode(
    name: &str,
    vec: Option<Vec<(u32, String)>>,
) -> Result<Option<u32>, ParsingErrorReason> {
    let vec = match vec {
        Some(v) => v,
        None => return Ok(None),
    };
    if vec.len() != 1 {
        return Err(ParsingErrorReason::SettingTooManyValues(
            name.to_owned(),
            super::map_tuples_to_second(vec),
        ));
    }
    let trimmed = vec[0].1.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let val = u32::from_str_radix(trimmed, 8).map_err(|_| {
        ParsingErrorReason::Generic(format!("{name} is not a valid octal mode: {trimmed}"))
    })?;
    if val > 0o7777 {
        return Err(ParsingErrorReason::Generic(format!(
            "{name} value out of range (must be a valid octal mode, max 7777): {trimmed}"
        )));
    }
    Ok(Some(val))
}

pub fn parse_exec_section(
    section: &mut ParsedSection,
) -> Result<ParsedExecSection, ParsingErrorReason> {
    let user = section.remove("USER");
    let group = section.remove("GROUP");
    let stdin = section.remove("STANDARDINPUT");
    let stdout = section.remove("STANDARDOUTPUT");
    let stderr = section.remove("STANDARDERROR");
    let supplementary_groups = section.remove("SUPPLEMENTARYGROUPS");
    let environment = section.remove("ENVIRONMENT");
    let environment_file = section.remove("ENVIRONMENTFILE");
    let working_directory = section.remove("WORKINGDIRECTORY");
    let state_directory = section.remove("STATEDIRECTORY");
    let logs_directory = section.remove("LOGSDIRECTORY");
    let logs_directory_mode = section.remove("LOGSDIRECTORYMODE");
    let runtime_directory = section.remove("RUNTIMEDIRECTORY");
    let runtime_directory_preserve = section.remove("RUNTIMEDIRECTORYPRESERVE");
    let tty_path = section.remove("TTYPATH");
    let tty_reset = section.remove("TTYRESET");
    let tty_vhangup = section.remove("TTYVHANGUP");
    let tty_vt_disallocate = section.remove("TTYVTDISALLOCATE");
    let ignore_sigpipe = section.remove("IGNORESIGPIPE");
    let utmp_identifier = section.remove("UTMPIDENTIFIER");
    let utmp_mode = section.remove("UTMPMODE");
    let import_credential = section.remove("IMPORTCREDENTIAL");
    let load_credential = section.remove("LOADCREDENTIAL");
    let load_credential_encrypted = section.remove("LOADCREDENTIALENCRYPTED");
    let set_credential = section.remove("SETCREDENTIAL");
    let set_credential_encrypted = section.remove("SETCREDENTIALENCRYPTED");
    let pass_environment = section.remove("PASSENVIRONMENT");
    let unset_environment = section.remove("UNSETENVIRONMENT");
    let oom_score_adjust = section.remove("OOMSCOREADJUST");
    let log_extra_fields = section.remove("LOGEXTRAFIELDS");
    let dynamic_user = section.remove("DYNAMICUSER");
    let system_call_filter = section.remove("SYSTEMCALLFILTER");
    let system_call_log = section.remove("SYSTEMCALLLOG");
    let protect_system = section.remove("PROTECTSYSTEM");
    let restrict_namespaces = section.remove("RESTRICTNAMESPACES");
    let restrict_realtime = section.remove("RESTRICTREALTIME");
    let restrict_address_families = section.remove("RESTRICTADDRESSFAMILIES");
    let restrict_file_systems = section.remove("RESTRICTFILESYSTEMS");
    let system_call_error_number = section.remove("SYSTEMCALLERRORNUMBER");
    let no_new_privileges = section.remove("NONEWPRIVILEGES");
    let protect_control_groups = section.remove("PROTECTCONTROLGROUPS");
    let protect_kernel_modules = section.remove("PROTECTKERNELMODULES");
    let restrict_suid_sgid = section.remove("RESTRICTSUIDSGID");
    let protect_kernel_logs = section.remove("PROTECTKERNELLOGS");
    let protect_kernel_tunables = section.remove("PROTECTKERNELTUNABLES");
    let capability_bounding_set = section.remove("CAPABILITYBOUNDINGSET");
    let ambient_capabilities = section.remove("AMBIENTCAPABILITIES");
    let protect_clock = section.remove("PROTECTCLOCK");
    let protect_home = section.remove("PROTECTHOME");
    let protect_hostname = section.remove("PROTECTHOSTNAME");
    let system_call_architectures = section.remove("SYSTEMCALLARCHITECTURES");
    let read_write_paths = section.remove("READWRITEPATHS");
    let memory_deny_write_execute = section.remove("MEMORYDENYWRITEEXECUTE");
    let lock_personality = section.remove("LOCKPERSONALITY");
    let protect_proc = section.remove("PROTECTPROC");
    let private_tmp = section.remove("PRIVATETMP");
    let private_devices = section.remove("PRIVATEDEVICES");
    let private_network = section.remove("PRIVATENETWORK");
    let private_users = section.remove("PRIVATEUSERS");
    let private_mounts = section.remove("PRIVATEMOUNTS");
    let io_scheduling_class = section.remove("IOSCHEDULINGCLASS");
    let io_scheduling_priority = section.remove("IOSCHEDULINGPRIORITY");
    let umask = section.remove("UMASK");
    let proc_subset = section.remove("PROCSUBSET");
    let nice = section.remove("NICE");
    let remove_ipc = section.remove("REMOVEIPC");
    let pam_name = section.remove("PAMNAME");

    // ── Resource limits (LimitXXX=) ──────────────────────────────────
    let limit_cpu = section.remove("LIMITCPU");
    let limit_core = section.remove("LIMITCORE");
    let limit_fsize = section.remove("LIMITFSIZE");
    let limit_data = section.remove("LIMITDATA");
    let limit_stack = section.remove("LIMITSTACK");
    let limit_rss = section.remove("LIMITRSS");
    let limit_nproc = section.remove("LIMITNPROC");
    let limit_memlock = section.remove("LIMITMEMLOCK");
    let limit_as = section.remove("LIMITAS");
    let limit_locks = section.remove("LIMITLOCKS");
    let limit_sigpending = section.remove("LIMITSIGPENDING");
    let limit_msgqueue = section.remove("LIMITMSGQUEUE");
    let limit_nice = section.remove("LIMITNICE");
    let limit_rtprio = section.remove("LIMITRTPRIO");
    let limit_rttime = section.remove("LIMITRTTIME");

    // ── Directory management ─────────────────────────────────────────
    let cache_directory = section.remove("CACHEDIRECTORY");
    let cache_directory_mode = section.remove("CACHEDIRECTORYMODE");
    let configuration_directory = section.remove("CONFIGURATIONDIRECTORY");
    let configuration_directory_mode = section.remove("CONFIGURATIONDIRECTORYMODE");
    let state_directory_mode = section.remove("STATEDIRECTORYMODE");
    let runtime_directory_mode = section.remove("RUNTIMEDIRECTORYMODE");

    // ── Path-based mount namespace directives ────────────────────────
    let read_only_paths = section.remove("READONLYPATHS");
    let inaccessible_paths = section.remove("INACCESSIBLEPATHS");
    let bind_paths = section.remove("BINDPATHS");
    let bind_read_only_paths = section.remove("BINDREADONLYPATHS");
    let temporary_file_system = section.remove("TEMPORARYFILESYSTEM");

    // ── Logging directives ───────────────────────────────────────────
    let syslog_identifier = section.remove("SYSLOGIDENTIFIER");
    let syslog_facility = section.remove("SYSLOGFACILITY");
    let syslog_level = section.remove("SYSLOGLEVEL");
    let syslog_level_prefix = section.remove("SYSLOGLEVELPREFIX");
    let log_level_max = section.remove("LOGLEVELMAX");
    let log_rate_limit_interval_sec = section.remove("LOGRATELIMITINTERVALSEC");
    let log_rate_limit_burst = section.remove("LOGRATELIMITBURST");
    let log_filter_patterns = section.remove("LOGFILTERPATTERNS");
    let log_namespace = section.remove("LOGNAMESPACE");

    // ── CPU scheduling directives ────────────────────────────────────
    let cpu_scheduling_policy = section.remove("CPUSCHEDULINGPOLICY");
    let cpu_scheduling_priority = section.remove("CPUSCHEDULINGPRIORITY");
    let cpu_scheduling_reset_on_fork = section.remove("CPUSCHEDULINGRESETONFORK");
    let cpu_affinity = section.remove("CPUAFFINITY");
    let numa_policy = section.remove("NUMAPOLICY");
    let numa_mask = section.remove("NUMAMASK");

    // ── Root filesystem / image directives ───────────────────────────
    let root_directory = section.remove("ROOTDIRECTORY");
    let root_image = section.remove("ROOTIMAGE");
    let root_image_options = section.remove("ROOTIMAGEOPTIONS");
    let root_hash = section.remove("ROOTHASH");
    let root_hash_signature = section.remove("ROOTHASHSIGNATURE");
    let root_verity = section.remove("ROOTVERITY");
    let root_ephemeral = section.remove("ROOTEPHEMERAL");
    let mount_api_vfs = section.remove("MOUNTAPIVFS");
    let extension_directories = section.remove("EXTENSIONDIRECTORIES");
    let extension_images = section.remove("EXTENSIONIMAGES");
    let mount_images = section.remove("MOUNTIMAGES");
    let bind_log_sockets = section.remove("BINDLOGSOCKETS");

    // ── Additional namespace directives ──────────────────────────────
    let private_ipc = section.remove("PRIVATEIPC");
    let private_pids = section.remove("PRIVATEPIDS");
    let ipc_namespace_path = section.remove("IPCNAMESPACEPATH");
    let network_namespace_path = section.remove("NETWORKNAMESPACEPATH");

    // ── Security directives ──────────────────────────────────────────
    let secure_bits = section.remove("SECUREBITS");
    let personality = section.remove("PERSONALITY");
    let selinux_context = section.remove("SELINUXCONTEXT");
    let apparmor_profile = section.remove("APPARMORPROFILE");
    let smack_process_label = section.remove("SMACKPROCESSLABEL");
    let keyring_mode = section.remove("KEYRINGMODE");
    let no_exec_paths = section.remove("NOEXECPATHS");
    let exec_paths = section.remove("EXECPATHS");
    let coredump_filter = section.remove("COREDUMPFILTER");

    // ── Misc directives ─────────────────────────────────────────────
    let timer_slack_nsec = section.remove("TIMERSLACKNSEC");
    let standard_input_text = section.remove("STANDARDINPUTTEXT");
    let standard_input_data = section.remove("STANDARDINPUTDATA");
    let set_login_environment = section.remove("SETLOGINENVIRONMENT");

    let user = match user {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                Some(vec.remove(0).1)
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "User".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };

    let group = match group {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                Some(vec.remove(0).1)
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Group".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };
    let stdin_option = match stdin {
        None => super::StandardInput::Null,
        Some(mut vec) => {
            if vec.len() == 1 {
                match vec.remove(0).1.to_lowercase().as_str() {
                    "null" | "" => super::StandardInput::Null,
                    "tty" => super::StandardInput::Tty,
                    "tty-force" => super::StandardInput::TtyForce,
                    "tty-fail" => super::StandardInput::TtyFail,
                    other => {
                        trace!("Unsupported StandardInput={}, falling back to null", other);
                        super::StandardInput::Null
                    }
                }
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "StandardInput".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                super::StandardInput::Null
            }
        }
    };

    let tty_path = match tty_path {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                Some(std::path::PathBuf::from(vec.remove(0).1))
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TTYPath".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };

    let tty_reset = match tty_reset {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TTYReset".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => false,
    };

    let tty_vhangup = match tty_vhangup {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TTYVHangup".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => false,
    };

    let tty_vt_disallocate = match tty_vt_disallocate {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "TTYVTDisallocate".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        None => false,
    };

    let ignore_sigpipe = match ignore_sigpipe {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "IgnoreSIGPIPE".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: true — SIGPIPE is ignored
        None => true,
    };

    let dynamic_user = match dynamic_user {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "DynamicUser".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let restrict_realtime = match restrict_realtime {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "RestrictRealtime".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let no_new_privileges = match no_new_privileges {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "NoNewPrivileges".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let protect_kernel_modules = match protect_kernel_modules {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "ProtectKernelModules".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let restrict_suid_sgid = match restrict_suid_sgid {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "RestrictSUIDSGID".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let protect_kernel_logs = match protect_kernel_logs {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "ProtectKernelLogs".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let protect_kernel_tunables = match protect_kernel_tunables {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "ProtectKernelTunables".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let protect_clock = match protect_clock {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "ProtectClock".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let protect_hostname = match protect_hostname {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "ProtectHostname".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let memory_deny_write_execute = match memory_deny_write_execute {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "MemoryDenyWriteExecute".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let lock_personality = match lock_personality {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "LockPersonality".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let private_tmp = match private_tmp {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "PrivateTmp".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let private_devices = match private_devices {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "PrivateDevices".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let private_network = match private_network {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "PrivateNetwork".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let private_users = match private_users {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "PrivateUsers".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let private_mounts = match private_mounts {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "PrivateMounts".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let protect_control_groups = match protect_control_groups {
        Some(vec) => {
            if vec.len() == 1 {
                string_to_bool(&vec[0].1)
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "ProtectControlGroups".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
        // systemd default: false
        None => false,
    };

    let stdout_path = match stdout {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                Some(vec.remove(0).1)
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Standardoutput".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };
    let stdout_path = if let Some(p) = stdout_path {
        Some(make_stdio_option(&p)?)
    } else {
        None
    };

    let stderr_path = match stderr {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                Some(vec.remove(0).1)
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "Standarderror".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };
    let stderr_path = if let Some(p) = stderr_path {
        Some(make_stdio_option(&p)?)
    } else {
        None
    };

    let supplementary_groups = match supplementary_groups {
        None => Vec::new(),
        Some(vec) => vec.iter().fold(Vec::new(), |mut acc, (_id, list)| {
            acc.extend(list.split(' ').map(std::string::ToString::to_string));
            acc
        }),
    };

    let environment = match environment {
        Some(vec) => {
            debug!("Env vec: {vec:?}");
            let mut all_vars = Vec::new();
            for (_idx, line) in &vec {
                let parsed = parse_environment(line)?;
                all_vars.extend(parsed.vars);
            }
            Some(EnvVars { vars: all_vars })
        }
        None => None,
    };

    // Parse EnvironmentFile= directives. A leading '-' on the path means the
    // file is optional (no error if it doesn't exist). Multiple directives are
    // allowed and each can list one file.
    let environment_files: Vec<(PathBuf, bool)> = match environment_file {
        Some(vec) => vec
            .into_iter()
            .map(|(_, val)| {
                let val = val.trim().to_string();
                if let Some(stripped) = val.strip_prefix('-') {
                    (PathBuf::from(stripped), true)
                } else {
                    (PathBuf::from(val), false)
                }
            })
            .collect(),
        None => Vec::new(),
    };

    let working_directory = match working_directory {
        None => None,
        Some(mut vec) => {
            if vec.len() == 1 {
                let dir = vec.remove(0).1;
                // Strip leading '-' prefix (makes it non-fatal if directory doesn't exist)
                let dir = dir.strip_prefix('-').unwrap_or(&dir);
                if dir == "~" {
                    // Home directory of the user; resolved later when user is known
                    Some(std::path::PathBuf::from("~"))
                } else {
                    Some(std::path::PathBuf::from(dir))
                }
            } else if vec.len() > 1 {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "WorkingDirectory".into(),
                    super::map_tuples_to_second(vec),
                ));
            } else {
                None
            }
        }
    };

    let state_directory = match state_directory {
        None => Vec::new(),
        Some(vec) => vec
            .into_iter()
            .flat_map(|(_, val)| {
                val.split_whitespace()
                    .map(|s| s.to_owned())
                    .collect::<Vec<_>>()
            })
            .collect(),
    };

    let logs_directory = match logs_directory {
        None => Vec::new(),
        Some(vec) => vec
            .into_iter()
            .flat_map(|(_, val)| {
                val.split_whitespace()
                    .map(|s| s.to_owned())
                    .collect::<Vec<_>>()
            })
            .collect(),
    };

    let logs_directory_mode: Option<u32> = match logs_directory_mode {
        None => None,
        Some(vec) => {
            if vec.len() == 1 {
                let trimmed = vec[0].1.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    let val = u32::from_str_radix(trimmed, 8).map_err(|_| {
                        ParsingErrorReason::Generic(format!(
                            "LogsDirectoryMode is not a valid octal mode: {trimmed}"
                        ))
                    })?;
                    if val > 0o7777 {
                        return Err(ParsingErrorReason::Generic(format!(
                            "LogsDirectoryMode value out of range (must be a valid octal mode, max 7777): {trimmed}"
                        )));
                    }
                    Some(val)
                }
            } else {
                return Err(ParsingErrorReason::SettingTooManyValues(
                    "LogsDirectoryMode".to_owned(),
                    super::map_tuples_to_second(vec),
                ));
            }
        }
    };

    let runtime_directory = match runtime_directory {
        None => Vec::new(),
        Some(vec) => vec
            .into_iter()
            .flat_map(|(_, val)| {
                val.split_whitespace()
                    .map(|s| s.to_owned())
                    .collect::<Vec<_>>()
            })
            .collect(),
    };

    Ok(ParsedExecSection {
        user,
        group,
        stdin_option,
        stdout_path,
        stderr_path,
        supplementary_groups,
        environment,
        environment_files,
        working_directory,
        state_directory,
        logs_directory,
        logs_directory_mode,
        runtime_directory,
        runtime_directory_preserve: match runtime_directory_preserve {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "no" | "false" | "0" => super::RuntimeDirectoryPreserve::No,
                        "yes" | "true" | "1" => super::RuntimeDirectoryPreserve::Yes,
                        "restart" => super::RuntimeDirectoryPreserve::Restart,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "RuntimeDirectoryPreserve".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RuntimeDirectoryPreserve".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::RuntimeDirectoryPreserve::default(),
        },
        oom_score_adjust: match oom_score_adjust {
            None => None,
            Some(vec) => {
                if vec.len() == 1 {
                    let val: i32 = vec[0].1.trim().parse().map_err(|_| {
                        ParsingErrorReason::UnknownSetting(
                            "OOMScoreAdjust".to_owned(),
                            vec[0].1.clone(),
                        )
                    })?;
                    Some(val.clamp(-1000, 1000))
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "OOMScoreAdjust".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        tty_path,
        tty_reset,
        tty_vhangup,
        tty_vt_disallocate,
        ignore_sigpipe,
        dynamic_user,
        utmp_identifier: match utmp_identifier {
            None => None,
            Some(mut vec) => {
                if vec.len() == 1 {
                    Some(vec.remove(0).1)
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "UtmpIdentifier".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        utmp_mode: match utmp_mode {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.to_lowercase().as_str() {
                        "init" => super::UtmpMode::Init,
                        "login" => super::UtmpMode::Login,
                        "user" => super::UtmpMode::User,
                        name => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "UtmpMode".to_owned(),
                                name.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "UtmpMode".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::UtmpMode::default(),
        },
        import_credentials: match import_credential {
            Some(vec) => vec
                .into_iter()
                .flat_map(|(_, val)| {
                    val.split_whitespace()
                        .map(|s| s.to_owned())
                        .collect::<Vec<_>>()
                })
                .collect(),
            None => Vec::new(),
        },
        load_credentials: match load_credential {
            Some(vec) => {
                // LoadCredential=ID:PATH — each directive specifies a credential
                // name and file path separated by a colon. Multiple directives
                // accumulate; an empty assignment resets the list.
                let mut creds = Vec::new();
                for (_idx, val) in vec {
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        creds.clear();
                        continue;
                    }
                    if let Some((id, path)) = trimmed.split_once(':') {
                        let id = id.trim();
                        let path = path.trim();
                        if !id.is_empty() && !path.is_empty() {
                            creds.push((id.to_owned(), path.to_owned()));
                        }
                    }
                }
                creds
            }
            None => Vec::new(),
        },
        load_credentials_encrypted: match load_credential_encrypted {
            Some(vec) => {
                // LoadCredentialEncrypted=ID:PATH — same format as LoadCredential
                // but the file content is expected to be encrypted. We parse it
                // identically; decryption is handled at runtime.
                let mut creds = Vec::new();
                for (_idx, val) in vec {
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        creds.clear();
                        continue;
                    }
                    if let Some((id, path)) = trimmed.split_once(':') {
                        let id = id.trim();
                        let path = path.trim();
                        if !id.is_empty() && !path.is_empty() {
                            creds.push((id.to_owned(), path.to_owned()));
                        }
                    }
                }
                creds
            }
            None => Vec::new(),
        },
        set_credentials: match set_credential {
            Some(vec) => {
                // SetCredential=ID:DATA — each directive specifies a credential
                // name and inline data separated by a colon. The data part may
                // contain colons (only the first colon separates ID from DATA).
                // Multiple directives accumulate; an empty assignment resets.
                let mut creds = Vec::new();
                for (_idx, val) in vec {
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        creds.clear();
                        continue;
                    }
                    if let Some((id, data)) = trimmed.split_once(':') {
                        let id = id.trim();
                        if !id.is_empty() {
                            creds.push((id.to_owned(), data.to_owned()));
                        }
                    }
                }
                creds
            }
            None => Vec::new(),
        },
        set_credentials_encrypted: match set_credential_encrypted {
            Some(vec) => {
                // SetCredentialEncrypted=ID:DATA — same format as SetCredential
                // but the data is expected to be encrypted (base64). We parse it
                // identically; decryption is handled at runtime.
                let mut creds = Vec::new();
                for (_idx, val) in vec {
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        creds.clear();
                        continue;
                    }
                    if let Some((id, data)) = trimmed.split_once(':') {
                        let id = id.trim();
                        if !id.is_empty() {
                            creds.push((id.to_owned(), data.to_owned()));
                        }
                    }
                }
                creds
            }
            None => Vec::new(),
        },
        pass_environment: match pass_environment {
            Some(vec) => {
                // Each directive is a space-separated list of variable names
                // to pass from the system manager's environment. Multiple
                // directives accumulate; an empty assignment resets the list.
                let mut names = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        names.clear();
                        continue;
                    }
                    for name in trimmed.split_whitespace() {
                        if !name.is_empty() {
                            names.push(name.to_owned());
                        }
                    }
                }
                names
            }
            None => Vec::new(),
        },
        unset_environment: match unset_environment {
            Some(vec) => {
                // Each directive is a space-separated list of variable names
                // or VAR=VALUE assignments. Multiple directives accumulate.
                // Tokens may be optionally quoted with double quotes (same
                // rules as Environment=). If an empty string is assigned,
                // the list is reset.
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    // Split on whitespace, respecting double-quoted tokens
                    let mut chars = trimmed.chars().peekable();
                    while chars.peek().is_some() {
                        // skip whitespace
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek().is_none() {
                            break;
                        }
                        let mut token = String::new();
                        if chars.peek() == Some(&'"') {
                            // quoted token — consume until closing quote
                            chars.next(); // skip opening quote
                            while let Some(&c) = chars.peek() {
                                if c == '"' {
                                    chars.next();
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        } else {
                            // unquoted token — consume until whitespace
                            while let Some(&c) = chars.peek() {
                                if c.is_whitespace() {
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        }
                        if !token.is_empty() {
                            entries.push(token);
                        }
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        log_extra_fields: match log_extra_fields {
            Some(vec) => vec.into_iter().map(|(_, val)| val).collect(),
            None => Vec::new(),
        },
        protect_system: match protect_system {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "no" | "false" | "0" => super::ProtectSystem::No,
                        "yes" | "true" | "1" => super::ProtectSystem::Yes,
                        "full" => super::ProtectSystem::Full,
                        "strict" => super::ProtectSystem::Strict,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "ProtectSystem".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "ProtectSystem".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::ProtectSystem::default(),
        },
        restrict_namespaces: match restrict_namespaces {
            Some(vec) => {
                if vec.len() == 1 {
                    let raw = vec[0].1.trim();
                    match raw.to_lowercase().as_str() {
                        "yes" | "true" | "1" => super::RestrictNamespaces::Yes,
                        "no" | "false" | "0" | "" => super::RestrictNamespaces::No,
                        _ => {
                            // Space-separated list of namespace types, optionally
                            // prefixed with ~ for deny-list mode.
                            let trimmed = raw;
                            let (deny, rest) = if let Some(stripped) = trimmed.strip_prefix('~') {
                                (true, stripped.trim_start())
                            } else {
                                (false, trimmed)
                            };
                            let names: Vec<String> =
                                rest.split_whitespace().map(|s| s.to_lowercase()).collect();
                            if deny {
                                super::RestrictNamespaces::Deny(names)
                            } else {
                                super::RestrictNamespaces::Allow(names)
                            }
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RestrictNamespaces".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::RestrictNamespaces::default(),
        },
        restrict_realtime,
        restrict_address_families: match restrict_address_families {
            Some(vec) => {
                // Each directive is a space-separated list of address family
                // names (e.g. AF_UNIX, AF_INET, AF_INET6). A leading ~ on
                // the whole value means deny-list mode. Multiple directives
                // accumulate. An empty assignment resets the list.
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    // Split on whitespace, respecting double-quoted tokens
                    let mut chars = trimmed.chars().peekable();
                    while chars.peek().is_some() {
                        // skip whitespace
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek().is_none() {
                            break;
                        }
                        let mut token = String::new();
                        if chars.peek() == Some(&'"') {
                            // quoted token — consume until closing quote
                            chars.next(); // skip opening quote
                            while let Some(&c) = chars.peek() {
                                if c == '"' {
                                    chars.next();
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        } else {
                            // unquoted token — consume until whitespace
                            while let Some(&c) = chars.peek() {
                                if c.is_whitespace() {
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        }
                        if !token.is_empty() {
                            entries.push(token);
                        }
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        no_new_privileges,
        protect_control_groups,
        protect_kernel_modules,
        restrict_suid_sgid,
        protect_kernel_logs,
        protect_kernel_tunables,
        protect_clock,
        capability_bounding_set: match capability_bounding_set {
            Some(vec) => {
                // Each directive is a space-separated list of capability names
                // (e.g. CAP_NET_ADMIN, CAP_SYS_PTRACE). A leading ~ on the
                // whole value means deny-list mode. Multiple directives
                // accumulate. An empty assignment resets the list.
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    // Split on whitespace, respecting double-quoted tokens
                    let mut chars = trimmed.chars().peekable();
                    while chars.peek().is_some() {
                        // skip whitespace
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek().is_none() {
                            break;
                        }
                        let mut token = String::new();
                        if chars.peek() == Some(&'"') {
                            // quoted token — consume until closing quote
                            chars.next(); // skip opening quote
                            while let Some(&c) = chars.peek() {
                                if c == '"' {
                                    chars.next();
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        } else {
                            // unquoted token — consume until whitespace
                            while let Some(&c) = chars.peek() {
                                if c.is_whitespace() {
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        }
                        if !token.is_empty() {
                            entries.push(token);
                        }
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        ambient_capabilities: match ambient_capabilities {
            Some(vec) => {
                // Same semantics as CapabilityBoundingSet=: space-separated
                // capability names, ~ prefix for deny-list, multiple
                // directives accumulate, empty assignment resets.
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    let mut chars = trimmed.chars().peekable();
                    while chars.peek().is_some() {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek().is_none() {
                            break;
                        }
                        let mut token = String::new();
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            while let Some(&c) = chars.peek() {
                                if c == '"' {
                                    chars.next();
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        } else {
                            while let Some(&c) = chars.peek() {
                                if c.is_whitespace() {
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        }
                        if !token.is_empty() {
                            entries.push(token);
                        }
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        system_call_error_number: match system_call_error_number {
            None => None,
            Some(mut vec) => {
                if vec.len() == 1 {
                    let val = vec.remove(0).1;
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "SystemCallErrorNumber".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        system_call_filter: match system_call_filter {
            Some(vec) => {
                // Each directive is a space-separated list of syscall names
                // or @group names. A leading ~ on the whole value means
                // deny-list mode; without it, allow-list mode. Multiple
                // directives accumulate. An empty assignment resets the list.
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    // Split on whitespace, respecting double-quoted tokens
                    let mut chars = trimmed.chars().peekable();
                    while chars.peek().is_some() {
                        // skip whitespace
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek().is_none() {
                            break;
                        }
                        let mut token = String::new();
                        if chars.peek() == Some(&'"') {
                            // quoted token — consume until closing quote
                            chars.next(); // skip opening quote
                            while let Some(&c) = chars.peek() {
                                if c == '"' {
                                    chars.next();
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        } else {
                            // unquoted token — consume until whitespace
                            while let Some(&c) = chars.peek() {
                                if c.is_whitespace() {
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        }
                        if !token.is_empty() {
                            entries.push(token);
                        }
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        system_call_log: match system_call_log {
            Some(vec) => {
                // Same syntax as SystemCallFilter=: space-separated syscall
                // names or @group names. A leading ~ means "log everything
                // except these". Multiple directives accumulate; an empty
                // assignment resets the list. See systemd.exec(5).
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    let mut chars = trimmed.chars().peekable();
                    while chars.peek().is_some() {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek().is_none() {
                            break;
                        }
                        let mut token = String::new();
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            while let Some(&c) = chars.peek() {
                                if c == '"' {
                                    chars.next();
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        } else {
                            while let Some(&c) = chars.peek() {
                                if c.is_whitespace() {
                                    break;
                                }
                                token.push(c);
                                chars.next();
                            }
                        }
                        if !token.is_empty() {
                            entries.push(token);
                        }
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        restrict_file_systems: {
            // Space-separated list of filesystem type names (e.g. ext4,
            // tmpfs, proc). A leading ~ means deny-list mode. Multiple
            // directives accumulate; an empty assignment resets the list.
            // Requires BPF LSM for runtime enforcement. See systemd.exec(5).
            let mut entries = Vec::new();
            if let Some(vec) = restrict_file_systems {
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        entries.clear();
                        continue;
                    }
                    for token in trimmed.split_whitespace() {
                        if !token.is_empty() {
                            entries.push(token.to_owned());
                        }
                    }
                }
            }
            entries
        },
        protect_home: match protect_home {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "no" | "false" | "0" => super::ProtectHome::No,
                        "yes" | "true" | "1" => super::ProtectHome::Yes,
                        "read-only" => super::ProtectHome::ReadOnly,
                        "tmpfs" => super::ProtectHome::Tmpfs,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "ProtectHome".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "ProtectHome".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::ProtectHome::default(),
        },
        protect_hostname,
        system_call_architectures: match system_call_architectures {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    for token in trimmed.split_whitespace() {
                        entries.push(token.to_owned());
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        read_write_paths: match read_write_paths {
            Some(vec) => {
                let mut entries = Vec::new();
                for (_idx, line) in &vec {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Empty string resets the list
                        entries.clear();
                        continue;
                    }
                    for token in trimmed.split_whitespace() {
                        entries.push(token.to_owned());
                    }
                }
                entries
            }
            None => Vec::new(),
        },
        memory_deny_write_execute,
        lock_personality,
        protect_proc: match protect_proc {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "default" | "" => super::ProtectProc::Default,
                        "noaccess" => super::ProtectProc::Noaccess,
                        "invisible" => super::ProtectProc::Invisible,
                        "ptraceable" => super::ProtectProc::Ptraceable,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "ProtectProc".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "ProtectProc".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::ProtectProc::default(),
        },
        private_tmp,
        private_devices,
        private_network,
        private_users,
        private_mounts,
        io_scheduling_class: match io_scheduling_class {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "none" | "0" | "" => super::IOSchedulingClass::None,
                        "realtime" | "1" => super::IOSchedulingClass::Realtime,
                        "best-effort" | "2" => super::IOSchedulingClass::BestEffort,
                        "idle" | "3" => super::IOSchedulingClass::Idle,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "IOSchedulingClass".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "IOSchedulingClass".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    super::IOSchedulingClass::default()
                }
            }
            None => super::IOSchedulingClass::default(),
        },
        io_scheduling_priority: match io_scheduling_priority {
            None => None,
            Some(vec) => {
                if vec.len() == 1 {
                    let val: u8 = vec[0].1.trim().parse().map_err(|_| {
                        ParsingErrorReason::UnknownSetting(
                            "IOSchedulingPriority".to_owned(),
                            vec[0].1.clone(),
                        )
                    })?;
                    if val > 7 {
                        return Err(ParsingErrorReason::UnknownSetting(
                            "IOSchedulingPriority".to_owned(),
                            format!("{val} (must be 0-7)"),
                        ));
                    }
                    Some(val)
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "IOSchedulingPriority".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        proc_subset: match proc_subset {
            Some(vec) => {
                if vec.len() == 1 {
                    match vec[0].1.trim().to_lowercase().as_str() {
                        "all" | "" => super::ProcSubset::All,
                        "pid" => super::ProcSubset::Pid,
                        other => {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "ProcSubset".to_owned(),
                                other.to_owned(),
                            ));
                        }
                    }
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "ProcSubset".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            None => super::ProcSubset::default(),
        },
        nice: match nice {
            None => None,
            Some(vec) => {
                if vec.len() == 1 {
                    let trimmed = vec[0].1.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        let val: i32 = trimmed.parse().map_err(|_| {
                            ParsingErrorReason::UnknownSetting("Nice".to_owned(), vec[0].1.clone())
                        })?;
                        if !(-20..=19).contains(&val) {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "Nice".to_owned(),
                                format!("{val} (must be -20 to 19)"),
                            ));
                        }
                        Some(val)
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "Nice".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        remove_ipc: match remove_ipc {
            Some(vec) => {
                if vec.len() == 1 {
                    string_to_bool(&vec[0].1)
                } else {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "RemoveIPC".to_owned(),
                        super::map_tuples_to_second(vec),
                    ));
                }
            }
            // systemd default: false
            None => false,
        },
        umask: match umask {
            None => None,
            Some(vec) => {
                if vec.len() == 1 {
                    let trimmed = vec[0].1.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        let val = u32::from_str_radix(trimmed, 8).map_err(|_| {
                            ParsingErrorReason::UnknownSetting("UMask".to_owned(), vec[0].1.clone())
                        })?;
                        if val > 0o7777 {
                            return Err(ParsingErrorReason::UnknownSetting(
                                "UMask".to_owned(),
                                format!(
                                    "{} (must be a valid octal mode, max 7777)",
                                    vec[0].1.trim()
                                ),
                            ));
                        }
                        Some(val)
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "UMask".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        pam_name: match pam_name {
            None => None,
            Some(mut vec) => {
                if vec.len() == 1 {
                    let val = vec.remove(0).1;
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "PAMName".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },

        // ── Resource limits (LimitXXX=) ──────────────────────────────
        // Byte-oriented limits accept K/M/G/T/P/E suffixes; non-byte limits
        // accept plain integers only.
        limit_cpu: parse_resource_limit("LimitCPU", limit_cpu, false)?,
        limit_core: parse_resource_limit("LimitCORE", limit_core, true)?,
        limit_fsize: parse_resource_limit("LimitFSIZE", limit_fsize, true)?,
        limit_data: parse_resource_limit("LimitDATA", limit_data, true)?,
        limit_stack: parse_resource_limit("LimitSTACK", limit_stack, true)?,
        limit_rss: parse_resource_limit("LimitRSS", limit_rss, true)?,
        limit_nproc: parse_resource_limit("LimitNPROC", limit_nproc, false)?,
        limit_memlock: parse_resource_limit("LimitMEMLOCK", limit_memlock, true)?,
        limit_as: parse_resource_limit("LimitAS", limit_as, true)?,
        limit_locks: parse_resource_limit("LimitLOCKS", limit_locks, false)?,
        limit_sigpending: parse_resource_limit("LimitSIGPENDING", limit_sigpending, false)?,
        limit_msgqueue: parse_resource_limit("LimitMSGQUEUE", limit_msgqueue, true)?,
        limit_nice: parse_resource_limit("LimitNICE", limit_nice, false)?,
        limit_rtprio: parse_resource_limit("LimitRTPRIO", limit_rtprio, false)?,
        limit_rttime: parse_resource_limit("LimitRTTIME", limit_rttime, false)?,

        // ── Directory management ─────────────────────────────────────
        cache_directory: parse_space_separated_list(cache_directory),
        cache_directory_mode: parse_octal_mode("CacheDirectoryMode", cache_directory_mode)?,
        configuration_directory: parse_space_separated_list(configuration_directory),
        configuration_directory_mode: parse_octal_mode(
            "ConfigurationDirectoryMode",
            configuration_directory_mode,
        )?,
        state_directory_mode: parse_octal_mode("StateDirectoryMode", state_directory_mode)?,
        runtime_directory_mode: parse_octal_mode("RuntimeDirectoryMode", runtime_directory_mode)?,

        // ── Path-based mount namespace directives ────────────────────
        read_only_paths: parse_space_separated_list(read_only_paths),
        inaccessible_paths: parse_space_separated_list(inaccessible_paths),
        bind_paths: parse_space_separated_list(bind_paths),
        bind_read_only_paths: parse_space_separated_list(bind_read_only_paths),
        temporary_file_system: parse_space_separated_list(temporary_file_system),

        // ── Logging directives ───────────────────────────────────────
        syslog_identifier: match syslog_identifier {
            None => None,
            Some(mut vec) => {
                if vec.len() == 1 {
                    let val = vec.remove(0).1;
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "SyslogIdentifier".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        syslog_facility: parse_optional_single_string("SyslogFacility", syslog_facility)?,
        syslog_level: parse_optional_single_string("SyslogLevel", syslog_level)?,
        syslog_level_prefix: parse_optional_bool("SyslogLevelPrefix", syslog_level_prefix)?,
        log_level_max: parse_optional_single_string("LogLevelMax", log_level_max)?,
        log_rate_limit_interval_sec: parse_optional_single_string(
            "LogRateLimitIntervalSec",
            log_rate_limit_interval_sec,
        )?,
        log_rate_limit_burst: match log_rate_limit_burst {
            None => None,
            Some(vec) => {
                if vec.len() == 1 {
                    let trimmed = vec[0].1.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.parse::<u32>().map_err(|_| {
                            ParsingErrorReason::Generic(format!(
                                "LogRateLimitBurst is not valid: {trimmed}"
                            ))
                        })?)
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "LogRateLimitBurst".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        log_filter_patterns: parse_space_separated_list(log_filter_patterns),
        log_namespace: parse_optional_single_string("LogNamespace", log_namespace)?,

        // ── CPU scheduling directives ────────────────────────────────
        cpu_scheduling_policy: parse_optional_single_string(
            "CPUSchedulingPolicy",
            cpu_scheduling_policy,
        )?,
        cpu_scheduling_priority: match cpu_scheduling_priority {
            None => None,
            Some(vec) => {
                if vec.len() == 1 {
                    let trimmed = vec[0].1.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.parse::<u32>().map_err(|_| {
                            ParsingErrorReason::Generic(format!(
                                "CPUSchedulingPriority is not valid: {trimmed}"
                            ))
                        })?)
                    }
                } else if vec.len() > 1 {
                    return Err(ParsingErrorReason::SettingTooManyValues(
                        "CPUSchedulingPriority".into(),
                        super::map_tuples_to_second(vec),
                    ));
                } else {
                    None
                }
            }
        },
        cpu_scheduling_reset_on_fork: parse_optional_bool(
            "CPUSchedulingResetOnFork",
            cpu_scheduling_reset_on_fork,
        )?,
        cpu_affinity: parse_space_separated_list(cpu_affinity),
        numa_policy: parse_optional_single_string("NUMAPolicy", numa_policy)?,
        numa_mask: parse_optional_single_string("NUMAMask", numa_mask)?,

        // ── Root filesystem / image directives ───────────────────────
        root_directory: parse_optional_single_string("RootDirectory", root_directory)?,
        root_image: parse_optional_single_string("RootImage", root_image)?,
        root_image_options: parse_space_separated_list(root_image_options),
        root_hash: parse_optional_single_string("RootHash", root_hash)?,
        root_hash_signature: parse_optional_single_string(
            "RootHashSignature",
            root_hash_signature,
        )?,
        root_verity: parse_optional_single_string("RootVerity", root_verity)?,
        root_ephemeral: parse_optional_bool("RootEphemeral", root_ephemeral)?,
        mount_api_vfs: parse_optional_bool("MountAPIVFS", mount_api_vfs)?,
        extension_directories: parse_space_separated_list(extension_directories),
        extension_images: parse_space_separated_list(extension_images),
        mount_images: parse_space_separated_list(mount_images),
        bind_log_sockets: parse_optional_bool("BindLogSockets", bind_log_sockets)?,

        // ── Additional namespace directives ──────────────────────────
        private_ipc: parse_optional_bool("PrivateIPC", private_ipc)?,
        private_pids: parse_optional_bool("PrivatePIDs", private_pids)?,
        ipc_namespace_path: parse_optional_single_string("IPCNamespacePath", ipc_namespace_path)?,
        network_namespace_path: parse_optional_single_string(
            "NetworkNamespacePath",
            network_namespace_path,
        )?,

        // ── Security directives ──────────────────────────────────────
        secure_bits: parse_space_separated_list(secure_bits),
        personality: parse_optional_single_string("Personality", personality)?,
        selinux_context: parse_optional_single_string("SELinuxContext", selinux_context)?,
        apparmor_profile: parse_optional_single_string("AppArmorProfile", apparmor_profile)?,
        smack_process_label: parse_optional_single_string(
            "SmackProcessLabel",
            smack_process_label,
        )?,
        keyring_mode: parse_optional_single_string("KeyringMode", keyring_mode)?,
        no_exec_paths: parse_space_separated_list(no_exec_paths),
        exec_paths: parse_space_separated_list(exec_paths),
        coredump_filter: parse_optional_single_string("CoredumpFilter", coredump_filter)?,

        // ── Misc directives ─────────────────────────────────────────
        timer_slack_nsec: parse_optional_single_string("TimerSlackNSec", timer_slack_nsec)?,
        standard_input_text: parse_space_separated_list(standard_input_text),
        standard_input_data: parse_space_separated_list(standard_input_data),
        set_login_environment: parse_optional_bool("SetLoginEnvironment", set_login_environment)?,
    })
}

/// Parse a single optional string value from a directive that allows exactly one value.
fn parse_optional_single_string(
    name: &str,
    vec: Option<Vec<(u32, String)>>,
) -> Result<Option<String>, ParsingErrorReason> {
    match vec {
        None => Ok(None),
        Some(mut v) => {
            if v.len() == 1 {
                let trimmed = v.remove(0).1;
                let trimmed = trimmed.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed.to_owned()))
                }
            } else if v.len() > 1 {
                Err(ParsingErrorReason::SettingTooManyValues(
                    name.to_owned(),
                    super::map_tuples_to_second(v),
                ))
            } else {
                Ok(None)
            }
        }
    }
}

/// Parse a single optional boolean value from a directive.
fn parse_optional_bool(
    name: &str,
    vec: Option<Vec<(u32, String)>>,
) -> Result<Option<bool>, ParsingErrorReason> {
    match vec {
        None => Ok(None),
        Some(vec) => {
            if vec.len() == 1 {
                let trimmed = vec[0].1.trim().to_lowercase();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(string_to_bool(&trimmed)))
                }
            } else if vec.len() > 1 {
                Err(ParsingErrorReason::SettingTooManyValues(
                    name.to_owned(),
                    super::map_tuples_to_second(vec),
                ))
            } else {
                Ok(None)
            }
        }
    }
}

pub fn parse_install_section(
    mut section: ParsedSection,
) -> Result<ParsedInstallSection, ParsingErrorReason> {
    let wantedby = section.remove("WANTEDBY");
    let requiredby = section.remove("REQUIREDBY");
    let also = section.remove("ALSO");
    let alias = section.remove("ALIAS");
    let default_instance = section.remove("DEFAULTINSTANCE");

    for key in section.keys() {
        if key.starts_with("X-") {
            trace!("Silently ignoring vendor extension in [Install] section: {key}");
            continue;
        }
        trace!("Ignoring unsupported setting in [Install] section: {key}");
    }

    Ok(ParsedInstallSection {
        wanted_by: map_tuples_to_second(split_list_values(wantedby.unwrap_or_default())),
        required_by: map_tuples_to_second(split_list_values(requiredby.unwrap_or_default())),
        also: map_tuples_to_second(split_list_values(also.unwrap_or_default())),
        alias: map_tuples_to_second(split_list_values(alias.unwrap_or_default())),
        default_instance: default_instance.map(|x| x[0].1.clone()),
    })
}

pub fn get_file_list(path: &PathBuf) -> Result<Vec<std::fs::DirEntry>, ParsingErrorReason> {
    if !path.exists() {
        return Err(ParsingErrorReason::Generic(format!(
            "Path to services does not exist: {path:?}"
        )));
    }
    if !path.is_dir() {
        return Err(ParsingErrorReason::Generic(format!(
            "Path to services does not exist: {path:?}"
        )));
    }
    let mut files: Vec<_> = match std::fs::read_dir(path) {
        Ok(iter) => iter
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ParsingErrorReason::FileError(Box::new(e)))?,
        Err(e) => return Err(ParsingErrorReason::FileError(Box::new(e))),
    };
    files.sort_by_key(std::fs::DirEntry::path);

    Ok(files)
}

#[must_use]
pub fn parse_section(lines: &[&str]) -> ParsedSection {
    let mut entries: ParsedSection = HashMap::new();

    let mut entry_number = 0;
    for line in lines {
        //ignore comments
        if line.starts_with('#') {
            continue;
        }

        //check if this is a key value pair
        let Some(pos) = line.find('=') else {
            continue;
        };
        let (name, value) = line.split_at(pos);

        let value = value.trim_start_matches('=');
        let value = value.trim();
        let name = name.trim().to_uppercase();

        let vec = entries.entry(name).or_default();
        vec.push((entry_number, value.to_string()));
        entry_number += 1;
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{RLimitValue, ResourceLimit};

    // ── Helper to build the Option<Vec<(u32, String)>> input ──────────
    fn single_val(s: &str) -> Option<Vec<(u32, String)>> {
        Some(vec![(0, s.to_owned())])
    }

    fn multi_val(vals: &[&str]) -> Option<Vec<(u32, String)>> {
        Some(
            vals.iter()
                .enumerate()
                .map(|(i, s)| (i as u32, s.to_string()))
                .collect(),
        )
    }

    // ════════════════════════════════════════════════════════════════════
    // parse_rlimit_value
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn rlimit_value_plain_number() {
        let v = parse_rlimit_value("Test", "1024", false).unwrap();
        assert_eq!(v, RLimitValue::Value(1024));
    }

    #[test]
    fn rlimit_value_infinity() {
        let v = parse_rlimit_value("Test", "infinity", false).unwrap();
        assert_eq!(v, RLimitValue::Infinity);
    }

    #[test]
    fn rlimit_value_infinity_case_insensitive() {
        let v = parse_rlimit_value("Test", "INFINITY", false).unwrap();
        assert_eq!(v, RLimitValue::Infinity);
    }

    #[test]
    fn rlimit_value_byte_suffix_k() {
        let v = parse_rlimit_value("Test", "4K", true).unwrap();
        assert_eq!(v, RLimitValue::Value(4 * 1024));
    }

    #[test]
    fn rlimit_value_byte_suffix_k_lower() {
        let v = parse_rlimit_value("Test", "4k", true).unwrap();
        assert_eq!(v, RLimitValue::Value(4 * 1024));
    }

    #[test]
    fn rlimit_value_byte_suffix_m() {
        let v = parse_rlimit_value("Test", "8M", true).unwrap();
        assert_eq!(v, RLimitValue::Value(8 * 1024 * 1024));
    }

    #[test]
    fn rlimit_value_byte_suffix_g() {
        let v = parse_rlimit_value("Test", "2G", true).unwrap();
        assert_eq!(v, RLimitValue::Value(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn rlimit_value_byte_suffix_t() {
        let v = parse_rlimit_value("Test", "1T", true).unwrap();
        assert_eq!(v, RLimitValue::Value(1024u64 * 1024 * 1024 * 1024));
    }

    #[test]
    fn rlimit_value_byte_suffix_p() {
        let v = parse_rlimit_value("Test", "1P", true).unwrap();
        assert_eq!(v, RLimitValue::Value(1024u64 * 1024 * 1024 * 1024 * 1024));
    }

    #[test]
    fn rlimit_value_byte_suffix_e() {
        let v = parse_rlimit_value("Test", "1E", true).unwrap();
        assert_eq!(
            v,
            RLimitValue::Value(1024u64 * 1024 * 1024 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn rlimit_value_plain_number_with_byte_flag() {
        // No suffix should still work when byte_limit=true
        let v = parse_rlimit_value("Test", "65536", true).unwrap();
        assert_eq!(v, RLimitValue::Value(65536));
    }

    #[test]
    fn rlimit_value_suffix_ignored_when_not_byte_limit() {
        // When byte_limit=false, suffixes are not stripped — "4K" is invalid
        assert!(parse_rlimit_value("Test", "4K", false).is_err());
    }

    #[test]
    fn rlimit_value_invalid_string() {
        assert!(parse_rlimit_value("Test", "abc", false).is_err());
    }

    #[test]
    fn rlimit_value_empty_string() {
        assert!(parse_rlimit_value("Test", "", false).is_err());
    }

    // ════════════════════════════════════════════════════════════════════
    // parse_resource_limit
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn resource_limit_none_input() {
        let r = parse_resource_limit("LimitCORE", None, true).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn resource_limit_empty_string_resets() {
        let r = parse_resource_limit("LimitCORE", single_val(""), true).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn resource_limit_infinity() {
        let r = parse_resource_limit("LimitCORE", single_val("infinity"), true).unwrap();
        assert_eq!(
            r,
            Some(ResourceLimit {
                soft: RLimitValue::Infinity,
                hard: RLimitValue::Infinity,
            })
        );
    }

    #[test]
    fn resource_limit_single_value() {
        let r = parse_resource_limit("LimitNPROC", single_val("4096"), false).unwrap();
        assert_eq!(
            r,
            Some(ResourceLimit {
                soft: RLimitValue::Value(4096),
                hard: RLimitValue::Value(4096),
            })
        );
    }

    #[test]
    fn resource_limit_soft_hard_pair() {
        let r = parse_resource_limit("LimitNOFILE", single_val("1024:65536"), false).unwrap();
        assert_eq!(
            r,
            Some(ResourceLimit {
                soft: RLimitValue::Value(1024),
                hard: RLimitValue::Value(65536),
            })
        );
    }

    #[test]
    fn resource_limit_soft_hard_with_infinity() {
        let r = parse_resource_limit("LimitNOFILE", single_val("1024:infinity"), false).unwrap();
        assert_eq!(
            r,
            Some(ResourceLimit {
                soft: RLimitValue::Value(1024),
                hard: RLimitValue::Infinity,
            })
        );
    }

    #[test]
    fn resource_limit_byte_suffix_in_pair() {
        let r = parse_resource_limit("LimitFSIZE", single_val("1M:2G"), true).unwrap();
        assert_eq!(
            r,
            Some(ResourceLimit {
                soft: RLimitValue::Value(1024 * 1024),
                hard: RLimitValue::Value(2 * 1024 * 1024 * 1024),
            })
        );
    }

    #[test]
    fn resource_limit_too_many_values_errors() {
        let input = multi_val(&["100", "200"]);
        assert!(parse_resource_limit("LimitCORE", input, true).is_err());
    }

    #[test]
    fn resource_limit_whitespace_trimmed() {
        let r = parse_resource_limit("LimitCORE", single_val("  infinity  "), true).unwrap();
        assert_eq!(
            r,
            Some(ResourceLimit {
                soft: RLimitValue::Infinity,
                hard: RLimitValue::Infinity,
            })
        );
    }

    // ════════════════════════════════════════════════════════════════════
    // parse_space_separated_list
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn space_list_none_returns_empty() {
        let r = parse_space_separated_list(None);
        assert!(r.is_empty());
    }

    #[test]
    fn space_list_single_entry() {
        let r = parse_space_separated_list(single_val("/foo"));
        assert_eq!(r, vec!["/foo"]);
    }

    #[test]
    fn space_list_multiple_tokens_in_one_line() {
        let r = parse_space_separated_list(single_val("/foo /bar /baz"));
        assert_eq!(r, vec!["/foo", "/bar", "/baz"]);
    }

    #[test]
    fn space_list_accumulates_across_lines() {
        let r = parse_space_separated_list(multi_val(&["/foo", "/bar /baz"]));
        assert_eq!(r, vec!["/foo", "/bar", "/baz"]);
    }

    #[test]
    fn space_list_empty_resets() {
        // An empty line resets, then new entries accumulate
        let r = parse_space_separated_list(multi_val(&["/old", "", "/new"]));
        assert_eq!(r, vec!["/new"]);
    }

    #[test]
    fn space_list_empty_at_end_clears_all() {
        let r = parse_space_separated_list(multi_val(&["/old", ""]));
        assert!(r.is_empty());
    }

    #[test]
    fn space_list_whitespace_only_resets() {
        let r = parse_space_separated_list(multi_val(&["/old", "   ", "/new"]));
        assert_eq!(r, vec!["/new"]);
    }

    // ════════════════════════════════════════════════════════════════════
    // parse_octal_mode
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn octal_mode_none_returns_none() {
        let r = parse_octal_mode("TestMode", None).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn octal_mode_empty_string_returns_none() {
        let r = parse_octal_mode("TestMode", single_val("")).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn octal_mode_0755() {
        let r = parse_octal_mode("TestMode", single_val("0755")).unwrap();
        assert_eq!(r, Some(0o755));
    }

    #[test]
    fn octal_mode_0700() {
        let r = parse_octal_mode("TestMode", single_val("0700")).unwrap();
        assert_eq!(r, Some(0o700));
    }

    #[test]
    fn octal_mode_755_without_leading_zero() {
        let r = parse_octal_mode("TestMode", single_val("755")).unwrap();
        assert_eq!(r, Some(0o755));
    }

    #[test]
    fn octal_mode_max_7777() {
        let r = parse_octal_mode("TestMode", single_val("7777")).unwrap();
        assert_eq!(r, Some(0o7777));
    }

    #[test]
    fn octal_mode_out_of_range_errors() {
        // 10000 octal = 4096 decimal > 0o7777 = 4095
        assert!(parse_octal_mode("TestMode", single_val("10000")).is_err());
    }

    #[test]
    fn octal_mode_invalid_digit_errors() {
        // '9' is not a valid octal digit
        assert!(parse_octal_mode("TestMode", single_val("0789")).is_err());
    }

    #[test]
    fn octal_mode_non_numeric_errors() {
        assert!(parse_octal_mode("TestMode", single_val("rwxr-xr-x")).is_err());
    }

    #[test]
    fn octal_mode_too_many_values_errors() {
        let input = multi_val(&["0755", "0700"]);
        assert!(parse_octal_mode("TestMode", input).is_err());
    }

    #[test]
    fn octal_mode_whitespace_trimmed() {
        let r = parse_octal_mode("TestMode", single_val("  0755  ")).unwrap();
        assert_eq!(r, Some(0o755));
    }

    // ════════════════════════════════════════════════════════════════════
    // parse_optional_single_string
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn optional_single_string_none() {
        let r = parse_optional_single_string("Test", None).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn optional_single_string_empty() {
        let r = parse_optional_single_string("Test", single_val("")).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn optional_single_string_value() {
        let r = parse_optional_single_string("Test", single_val("hello")).unwrap();
        assert_eq!(r, Some("hello".to_owned()));
    }

    #[test]
    fn optional_single_string_whitespace_trimmed() {
        let r = parse_optional_single_string("Test", single_val("  hello  ")).unwrap();
        assert_eq!(r, Some("hello".to_owned()));
    }

    #[test]
    fn optional_single_string_too_many_values() {
        assert!(parse_optional_single_string("Test", multi_val(&["a", "b"])).is_err());
    }

    // ════════════════════════════════════════════════════════════════════
    // parse_optional_bool
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn optional_bool_none() {
        let r = parse_optional_bool("Test", None).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn optional_bool_empty() {
        let r = parse_optional_bool("Test", single_val("")).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn optional_bool_true_variants() {
        // string_to_bool recognizes: "yes", "true", "1" (case-insensitive)
        for val in &["true", "yes", "1", "True", "YES"] {
            let r = parse_optional_bool("Test", single_val(val)).unwrap();
            assert_eq!(r, Some(true), "expected true for '{val}'");
        }
    }

    #[test]
    fn optional_bool_false_variants() {
        // string_to_bool returns false for anything not "yes"/"true"/"1"
        for val in &["false", "no", "off", "0", "False", "NO"] {
            let r = parse_optional_bool("Test", single_val(val)).unwrap();
            assert_eq!(r, Some(false), "expected false for '{val}'");
        }
    }

    #[test]
    fn optional_bool_too_many_values() {
        assert!(parse_optional_bool("Test", multi_val(&["true", "false"])).is_err());
    }

    // ════════════════════════════════════════════════════════════════════
    // New exec directives — integration tests via parse_exec_section
    // ════════════════════════════════════════════════════════════════════

    /// Helper: build a ParsedSection from key=value lines, calling
    /// `parse_exec_section` on the result.
    fn exec_from_lines(lines: &[&str]) -> ParsedExecSection {
        let mut section = parse_section(lines);
        parse_exec_section(&mut section).expect("parse_exec_section failed")
    }

    // ── Logging directives ───────────────────────────────────────────

    #[test]
    fn exec_syslog_facility_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.syslog_facility, None);
    }

    #[test]
    fn exec_syslog_facility_daemon() {
        let s = exec_from_lines(&["SyslogFacility=daemon"]);
        assert_eq!(s.syslog_facility, Some("daemon".to_owned()));
    }

    #[test]
    fn exec_syslog_facility_local0() {
        let s = exec_from_lines(&["SyslogFacility=local0"]);
        assert_eq!(s.syslog_facility, Some("local0".to_owned()));
    }

    #[test]
    fn exec_syslog_level_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.syslog_level, None);
    }

    #[test]
    fn exec_syslog_level_debug() {
        let s = exec_from_lines(&["SyslogLevel=debug"]);
        assert_eq!(s.syslog_level, Some("debug".to_owned()));
    }

    #[test]
    fn exec_syslog_level_err() {
        let s = exec_from_lines(&["SyslogLevel=err"]);
        assert_eq!(s.syslog_level, Some("err".to_owned()));
    }

    #[test]
    fn exec_syslog_level_prefix_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.syslog_level_prefix, None);
    }

    #[test]
    fn exec_syslog_level_prefix_true() {
        let s = exec_from_lines(&["SyslogLevelPrefix=true"]);
        assert_eq!(s.syslog_level_prefix, Some(true));
    }

    #[test]
    fn exec_syslog_level_prefix_false() {
        let s = exec_from_lines(&["SyslogLevelPrefix=false"]);
        assert_eq!(s.syslog_level_prefix, Some(false));
    }

    #[test]
    fn exec_log_level_max_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.log_level_max, None);
    }

    #[test]
    fn exec_log_level_max_warning() {
        let s = exec_from_lines(&["LogLevelMax=warning"]);
        assert_eq!(s.log_level_max, Some("warning".to_owned()));
    }

    #[test]
    fn exec_log_rate_limit_interval_sec_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.log_rate_limit_interval_sec, None);
    }

    #[test]
    fn exec_log_rate_limit_interval_sec_value() {
        let s = exec_from_lines(&["LogRateLimitIntervalSec=30s"]);
        assert_eq!(s.log_rate_limit_interval_sec, Some("30s".to_owned()));
    }

    #[test]
    fn exec_log_rate_limit_burst_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.log_rate_limit_burst, None);
    }

    #[test]
    fn exec_log_rate_limit_burst_value() {
        let s = exec_from_lines(&["LogRateLimitBurst=10000"]);
        assert_eq!(s.log_rate_limit_burst, Some(10000));
    }

    #[test]
    fn exec_log_rate_limit_burst_zero() {
        let s = exec_from_lines(&["LogRateLimitBurst=0"]);
        assert_eq!(s.log_rate_limit_burst, Some(0));
    }

    #[test]
    fn exec_log_rate_limit_burst_invalid() {
        let lines: &[&str] = &["LogRateLimitBurst=abc"];
        let mut section = parse_section(lines);
        assert!(parse_exec_section(&mut section).is_err());
    }

    #[test]
    fn exec_log_filter_patterns_default() {
        let s = exec_from_lines(&[]);
        assert!(s.log_filter_patterns.is_empty());
    }

    #[test]
    fn exec_log_filter_patterns_single() {
        let s = exec_from_lines(&["LogFilterPatterns=~.*debug.*"]);
        assert_eq!(s.log_filter_patterns, vec!["~.*debug.*"]);
    }

    #[test]
    fn exec_log_filter_patterns_multiple() {
        let s = exec_from_lines(&["LogFilterPatterns=~.*debug.* .*error.*"]);
        assert_eq!(s.log_filter_patterns, vec!["~.*debug.*", ".*error.*"]);
    }

    #[test]
    fn exec_log_namespace_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.log_namespace, None);
    }

    #[test]
    fn exec_log_namespace_value() {
        let s = exec_from_lines(&["LogNamespace=myapp"]);
        assert_eq!(s.log_namespace, Some("myapp".to_owned()));
    }

    // ── CPU scheduling directives ────────────────────────────────────

    #[test]
    fn exec_cpu_scheduling_policy_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.cpu_scheduling_policy, None);
    }

    #[test]
    fn exec_cpu_scheduling_policy_fifo() {
        let s = exec_from_lines(&["CPUSchedulingPolicy=fifo"]);
        assert_eq!(s.cpu_scheduling_policy, Some("fifo".to_owned()));
    }

    #[test]
    fn exec_cpu_scheduling_policy_rr() {
        let s = exec_from_lines(&["CPUSchedulingPolicy=rr"]);
        assert_eq!(s.cpu_scheduling_policy, Some("rr".to_owned()));
    }

    #[test]
    fn exec_cpu_scheduling_policy_batch() {
        let s = exec_from_lines(&["CPUSchedulingPolicy=batch"]);
        assert_eq!(s.cpu_scheduling_policy, Some("batch".to_owned()));
    }

    #[test]
    fn exec_cpu_scheduling_policy_idle() {
        let s = exec_from_lines(&["CPUSchedulingPolicy=idle"]);
        assert_eq!(s.cpu_scheduling_policy, Some("idle".to_owned()));
    }

    #[test]
    fn exec_cpu_scheduling_policy_other() {
        let s = exec_from_lines(&["CPUSchedulingPolicy=other"]);
        assert_eq!(s.cpu_scheduling_policy, Some("other".to_owned()));
    }

    #[test]
    fn exec_cpu_scheduling_priority_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.cpu_scheduling_priority, None);
    }

    #[test]
    fn exec_cpu_scheduling_priority_value() {
        let s = exec_from_lines(&["CPUSchedulingPriority=50"]);
        assert_eq!(s.cpu_scheduling_priority, Some(50));
    }

    #[test]
    fn exec_cpu_scheduling_priority_max() {
        let s = exec_from_lines(&["CPUSchedulingPriority=99"]);
        assert_eq!(s.cpu_scheduling_priority, Some(99));
    }

    #[test]
    fn exec_cpu_scheduling_priority_min() {
        let s = exec_from_lines(&["CPUSchedulingPriority=1"]);
        assert_eq!(s.cpu_scheduling_priority, Some(1));
    }

    #[test]
    fn exec_cpu_scheduling_priority_invalid() {
        let lines: &[&str] = &["CPUSchedulingPriority=abc"];
        let mut section = parse_section(lines);
        assert!(parse_exec_section(&mut section).is_err());
    }

    #[test]
    fn exec_cpu_scheduling_reset_on_fork_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.cpu_scheduling_reset_on_fork, None);
    }

    #[test]
    fn exec_cpu_scheduling_reset_on_fork_true() {
        let s = exec_from_lines(&["CPUSchedulingResetOnFork=yes"]);
        assert_eq!(s.cpu_scheduling_reset_on_fork, Some(true));
    }

    #[test]
    fn exec_cpu_scheduling_reset_on_fork_false() {
        let s = exec_from_lines(&["CPUSchedulingResetOnFork=no"]);
        assert_eq!(s.cpu_scheduling_reset_on_fork, Some(false));
    }

    #[test]
    fn exec_cpu_affinity_default() {
        let s = exec_from_lines(&[]);
        assert!(s.cpu_affinity.is_empty());
    }

    #[test]
    fn exec_cpu_affinity_single() {
        let s = exec_from_lines(&["CPUAffinity=0"]);
        assert_eq!(s.cpu_affinity, vec!["0"]);
    }

    #[test]
    fn exec_cpu_affinity_range() {
        let s = exec_from_lines(&["CPUAffinity=0-3 8-11"]);
        assert_eq!(s.cpu_affinity, vec!["0-3", "8-11"]);
    }

    #[test]
    fn exec_cpu_affinity_multiple() {
        let s = exec_from_lines(&["CPUAffinity=0 1 2 3"]);
        assert_eq!(s.cpu_affinity, vec!["0", "1", "2", "3"]);
    }

    #[test]
    fn exec_cpu_affinity_empty_resets() {
        let s = exec_from_lines(&["CPUAffinity=0 1", "CPUAffinity="]);
        assert!(s.cpu_affinity.is_empty());
    }

    #[test]
    fn exec_numa_policy_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.numa_policy, None);
    }

    #[test]
    fn exec_numa_policy_bind() {
        let s = exec_from_lines(&["NUMAPolicy=bind"]);
        assert_eq!(s.numa_policy, Some("bind".to_owned()));
    }

    #[test]
    fn exec_numa_policy_interleave() {
        let s = exec_from_lines(&["NUMAPolicy=interleave"]);
        assert_eq!(s.numa_policy, Some("interleave".to_owned()));
    }

    #[test]
    fn exec_numa_policy_local() {
        let s = exec_from_lines(&["NUMAPolicy=local"]);
        assert_eq!(s.numa_policy, Some("local".to_owned()));
    }

    #[test]
    fn exec_numa_mask_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.numa_mask, None);
    }

    #[test]
    fn exec_numa_mask_value() {
        let s = exec_from_lines(&["NUMAMask=0-3"]);
        assert_eq!(s.numa_mask, Some("0-3".to_owned()));
    }

    // ── Root filesystem / image directives ────────────────────────────

    #[test]
    fn exec_root_directory_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.root_directory, None);
    }

    #[test]
    fn exec_root_directory_value() {
        let s = exec_from_lines(&["RootDirectory=/srv/myapp"]);
        assert_eq!(s.root_directory, Some("/srv/myapp".to_owned()));
    }

    #[test]
    fn exec_root_image_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.root_image, None);
    }

    #[test]
    fn exec_root_image_value() {
        let s = exec_from_lines(&["RootImage=/var/lib/machines/image.raw"]);
        assert_eq!(s.root_image, Some("/var/lib/machines/image.raw".to_owned()));
    }

    #[test]
    fn exec_root_image_options_default() {
        let s = exec_from_lines(&[]);
        assert!(s.root_image_options.is_empty());
    }

    #[test]
    fn exec_root_image_options_value() {
        let s = exec_from_lines(&["RootImageOptions=root:ro,nodev"]);
        assert_eq!(s.root_image_options, vec!["root:ro,nodev"]);
    }

    #[test]
    fn exec_root_hash_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.root_hash, None);
    }

    #[test]
    fn exec_root_hash_value() {
        let s = exec_from_lines(&["RootHash=abc123def456"]);
        assert_eq!(s.root_hash, Some("abc123def456".to_owned()));
    }

    #[test]
    fn exec_root_hash_signature_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.root_hash_signature, None);
    }

    #[test]
    fn exec_root_hash_signature_file() {
        let s = exec_from_lines(&["RootHashSignature=/path/to/sig.p7s"]);
        assert_eq!(s.root_hash_signature, Some("/path/to/sig.p7s".to_owned()));
    }

    #[test]
    fn exec_root_hash_signature_inline() {
        let s = exec_from_lines(&["RootHashSignature=base64:AAAA"]);
        assert_eq!(s.root_hash_signature, Some("base64:AAAA".to_owned()));
    }

    #[test]
    fn exec_root_verity_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.root_verity, None);
    }

    #[test]
    fn exec_root_verity_value() {
        let s = exec_from_lines(&["RootVerity=/path/to/verity"]);
        assert_eq!(s.root_verity, Some("/path/to/verity".to_owned()));
    }

    #[test]
    fn exec_root_ephemeral_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.root_ephemeral, None);
    }

    #[test]
    fn exec_root_ephemeral_true() {
        let s = exec_from_lines(&["RootEphemeral=yes"]);
        assert_eq!(s.root_ephemeral, Some(true));
    }

    #[test]
    fn exec_root_ephemeral_false() {
        let s = exec_from_lines(&["RootEphemeral=no"]);
        assert_eq!(s.root_ephemeral, Some(false));
    }

    #[test]
    fn exec_mount_api_vfs_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.mount_api_vfs, None);
    }

    #[test]
    fn exec_mount_api_vfs_true() {
        let s = exec_from_lines(&["MountAPIVFS=yes"]);
        assert_eq!(s.mount_api_vfs, Some(true));
    }

    #[test]
    fn exec_extension_directories_default() {
        let s = exec_from_lines(&[]);
        assert!(s.extension_directories.is_empty());
    }

    #[test]
    fn exec_extension_directories_value() {
        let s = exec_from_lines(&["ExtensionDirectories=/run/extensions/myext"]);
        assert_eq!(s.extension_directories, vec!["/run/extensions/myext"]);
    }

    #[test]
    fn exec_extension_images_default() {
        let s = exec_from_lines(&[]);
        assert!(s.extension_images.is_empty());
    }

    #[test]
    fn exec_extension_images_value() {
        let s = exec_from_lines(&["ExtensionImages=/var/lib/extensions/ext.raw"]);
        assert_eq!(s.extension_images, vec!["/var/lib/extensions/ext.raw"]);
    }

    #[test]
    fn exec_mount_images_default() {
        let s = exec_from_lines(&[]);
        assert!(s.mount_images.is_empty());
    }

    #[test]
    fn exec_mount_images_value() {
        let s = exec_from_lines(&["MountImages=/image.raw:/mnt:ro,nodev"]);
        assert_eq!(s.mount_images, vec!["/image.raw:/mnt:ro,nodev"]);
    }

    #[test]
    fn exec_bind_log_sockets_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.bind_log_sockets, None);
    }

    #[test]
    fn exec_bind_log_sockets_true() {
        let s = exec_from_lines(&["BindLogSockets=yes"]);
        assert_eq!(s.bind_log_sockets, Some(true));
    }

    // ── Additional namespace directives ──────────────────────────────

    #[test]
    fn exec_private_ipc_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.private_ipc, None);
    }

    #[test]
    fn exec_private_ipc_true() {
        let s = exec_from_lines(&["PrivateIPC=yes"]);
        assert_eq!(s.private_ipc, Some(true));
    }

    #[test]
    fn exec_private_ipc_false() {
        let s = exec_from_lines(&["PrivateIPC=no"]);
        assert_eq!(s.private_ipc, Some(false));
    }

    #[test]
    fn exec_private_pids_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.private_pids, None);
    }

    #[test]
    fn exec_private_pids_true() {
        let s = exec_from_lines(&["PrivatePIDs=yes"]);
        assert_eq!(s.private_pids, Some(true));
    }

    #[test]
    fn exec_ipc_namespace_path_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.ipc_namespace_path, None);
    }

    #[test]
    fn exec_ipc_namespace_path_value() {
        let s = exec_from_lines(&["IPCNamespacePath=/proc/42/ns/ipc"]);
        assert_eq!(s.ipc_namespace_path, Some("/proc/42/ns/ipc".to_owned()));
    }

    #[test]
    fn exec_network_namespace_path_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.network_namespace_path, None);
    }

    #[test]
    fn exec_network_namespace_path_value() {
        let s = exec_from_lines(&["NetworkNamespacePath=/run/netns/mynet"]);
        assert_eq!(
            s.network_namespace_path,
            Some("/run/netns/mynet".to_owned())
        );
    }

    // ── Security directives ──────────────────────────────────────────

    #[test]
    fn exec_secure_bits_default() {
        let s = exec_from_lines(&[]);
        assert!(s.secure_bits.is_empty());
    }

    #[test]
    fn exec_secure_bits_single() {
        let s = exec_from_lines(&["SecureBits=keep-caps"]);
        assert_eq!(s.secure_bits, vec!["keep-caps"]);
    }

    #[test]
    fn exec_secure_bits_multiple() {
        let s = exec_from_lines(&["SecureBits=keep-caps noroot no-setuid-fixup"]);
        assert_eq!(
            s.secure_bits,
            vec!["keep-caps", "noroot", "no-setuid-fixup"]
        );
    }

    #[test]
    fn exec_personality_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.personality, None);
    }

    #[test]
    fn exec_personality_x86() {
        let s = exec_from_lines(&["Personality=x86"]);
        assert_eq!(s.personality, Some("x86".to_owned()));
    }

    #[test]
    fn exec_personality_x86_64() {
        let s = exec_from_lines(&["Personality=x86-64"]);
        assert_eq!(s.personality, Some("x86-64".to_owned()));
    }

    #[test]
    fn exec_selinux_context_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.selinux_context, None);
    }

    #[test]
    fn exec_selinux_context_value() {
        let s = exec_from_lines(&["SELinuxContext=system_u:system_r:httpd_t:s0"]);
        assert_eq!(
            s.selinux_context,
            Some("system_u:system_r:httpd_t:s0".to_owned())
        );
    }

    #[test]
    fn exec_selinux_context_optional_prefix() {
        let s = exec_from_lines(&["SELinuxContext=-system_u:system_r:httpd_t:s0"]);
        assert_eq!(
            s.selinux_context,
            Some("-system_u:system_r:httpd_t:s0".to_owned())
        );
    }

    #[test]
    fn exec_apparmor_profile_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.apparmor_profile, None);
    }

    #[test]
    fn exec_apparmor_profile_value() {
        let s = exec_from_lines(&["AppArmorProfile=usr.sbin.mysqld"]);
        assert_eq!(s.apparmor_profile, Some("usr.sbin.mysqld".to_owned()));
    }

    #[test]
    fn exec_apparmor_profile_optional_prefix() {
        let s = exec_from_lines(&["AppArmorProfile=-usr.sbin.mysqld"]);
        assert_eq!(s.apparmor_profile, Some("-usr.sbin.mysqld".to_owned()));
    }

    #[test]
    fn exec_smack_process_label_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.smack_process_label, None);
    }

    #[test]
    fn exec_smack_process_label_value() {
        let s = exec_from_lines(&["SmackProcessLabel=MyLabel"]);
        assert_eq!(s.smack_process_label, Some("MyLabel".to_owned()));
    }

    #[test]
    fn exec_keyring_mode_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.keyring_mode, None);
    }

    #[test]
    fn exec_keyring_mode_private() {
        let s = exec_from_lines(&["KeyringMode=private"]);
        assert_eq!(s.keyring_mode, Some("private".to_owned()));
    }

    #[test]
    fn exec_keyring_mode_shared() {
        let s = exec_from_lines(&["KeyringMode=shared"]);
        assert_eq!(s.keyring_mode, Some("shared".to_owned()));
    }

    #[test]
    fn exec_keyring_mode_inherit() {
        let s = exec_from_lines(&["KeyringMode=inherit"]);
        assert_eq!(s.keyring_mode, Some("inherit".to_owned()));
    }

    #[test]
    fn exec_no_exec_paths_default() {
        let s = exec_from_lines(&[]);
        assert!(s.no_exec_paths.is_empty());
    }

    #[test]
    fn exec_no_exec_paths_single() {
        let s = exec_from_lines(&["NoExecPaths=/tmp"]);
        assert_eq!(s.no_exec_paths, vec!["/tmp"]);
    }

    #[test]
    fn exec_no_exec_paths_multiple() {
        let s = exec_from_lines(&["NoExecPaths=/tmp /var/tmp /dev/shm"]);
        assert_eq!(s.no_exec_paths, vec!["/tmp", "/var/tmp", "/dev/shm"]);
    }

    #[test]
    fn exec_exec_paths_default() {
        let s = exec_from_lines(&[]);
        assert!(s.exec_paths.is_empty());
    }

    #[test]
    fn exec_exec_paths_single() {
        let s = exec_from_lines(&["ExecPaths=/usr/bin"]);
        assert_eq!(s.exec_paths, vec!["/usr/bin"]);
    }

    #[test]
    fn exec_exec_paths_multiple() {
        let s = exec_from_lines(&["ExecPaths=/usr/bin /usr/sbin /bin"]);
        assert_eq!(s.exec_paths, vec!["/usr/bin", "/usr/sbin", "/bin"]);
    }

    #[test]
    fn exec_coredump_filter_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.coredump_filter, None);
    }

    #[test]
    fn exec_coredump_filter_value() {
        let s = exec_from_lines(&["CoredumpFilter=private-anonymous shared-anonymous"]);
        assert_eq!(
            s.coredump_filter,
            Some("private-anonymous shared-anonymous".to_owned())
        );
    }

    #[test]
    fn exec_coredump_filter_hex() {
        let s = exec_from_lines(&["CoredumpFilter=0x33"]);
        assert_eq!(s.coredump_filter, Some("0x33".to_owned()));
    }

    // ── Misc directives ─────────────────────────────────────────────

    #[test]
    fn exec_timer_slack_nsec_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.timer_slack_nsec, None);
    }

    #[test]
    fn exec_timer_slack_nsec_value() {
        let s = exec_from_lines(&["TimerSlackNSec=50000"]);
        assert_eq!(s.timer_slack_nsec, Some("50000".to_owned()));
    }

    #[test]
    fn exec_standard_input_text_default() {
        let s = exec_from_lines(&[]);
        assert!(s.standard_input_text.is_empty());
    }

    #[test]
    fn exec_standard_input_text_value() {
        let s = exec_from_lines(&["StandardInputText=hello"]);
        assert_eq!(s.standard_input_text, vec!["hello"]);
    }

    #[test]
    fn exec_standard_input_data_default() {
        let s = exec_from_lines(&[]);
        assert!(s.standard_input_data.is_empty());
    }

    #[test]
    fn exec_standard_input_data_value() {
        let s = exec_from_lines(&["StandardInputData=SGVsbG8gV29ybGQK"]);
        assert_eq!(s.standard_input_data, vec!["SGVsbG8gV29ybGQK"]);
    }

    #[test]
    fn exec_set_login_environment_default() {
        let s = exec_from_lines(&[]);
        assert_eq!(s.set_login_environment, None);
    }

    #[test]
    fn exec_set_login_environment_true() {
        let s = exec_from_lines(&["SetLoginEnvironment=yes"]);
        assert_eq!(s.set_login_environment, Some(true));
    }

    #[test]
    fn exec_set_login_environment_false() {
        let s = exec_from_lines(&["SetLoginEnvironment=no"]);
        assert_eq!(s.set_login_environment, Some(false));
    }

    // ── Combined / integration tests ─────────────────────────────────

    #[test]
    fn exec_all_new_logging_directives_together() {
        let s = exec_from_lines(&[
            "SyslogFacility=daemon",
            "SyslogLevel=info",
            "SyslogLevelPrefix=true",
            "LogLevelMax=notice",
            "LogRateLimitIntervalSec=10s",
            "LogRateLimitBurst=500",
            "LogFilterPatterns=~.*spam.*",
            "LogNamespace=myns",
        ]);
        assert_eq!(s.syslog_facility, Some("daemon".to_owned()));
        assert_eq!(s.syslog_level, Some("info".to_owned()));
        assert_eq!(s.syslog_level_prefix, Some(true));
        assert_eq!(s.log_level_max, Some("notice".to_owned()));
        assert_eq!(s.log_rate_limit_interval_sec, Some("10s".to_owned()));
        assert_eq!(s.log_rate_limit_burst, Some(500));
        assert_eq!(s.log_filter_patterns, vec!["~.*spam.*"]);
        assert_eq!(s.log_namespace, Some("myns".to_owned()));
    }

    #[test]
    fn exec_all_new_cpu_directives_together() {
        let s = exec_from_lines(&[
            "CPUSchedulingPolicy=rr",
            "CPUSchedulingPriority=50",
            "CPUSchedulingResetOnFork=yes",
            "CPUAffinity=0-3",
            "NUMAPolicy=bind",
            "NUMAMask=0-1",
        ]);
        assert_eq!(s.cpu_scheduling_policy, Some("rr".to_owned()));
        assert_eq!(s.cpu_scheduling_priority, Some(50));
        assert_eq!(s.cpu_scheduling_reset_on_fork, Some(true));
        assert_eq!(s.cpu_affinity, vec!["0-3"]);
        assert_eq!(s.numa_policy, Some("bind".to_owned()));
        assert_eq!(s.numa_mask, Some("0-1".to_owned()));
    }

    #[test]
    fn exec_all_new_root_directives_together() {
        let s = exec_from_lines(&[
            "RootDirectory=/srv/app",
            "RootImage=/var/lib/app.raw",
            "RootImageOptions=root:ro",
            "RootHash=deadbeef",
            "RootHashSignature=base64:AAAA",
            "RootVerity=/var/lib/app.verity",
            "RootEphemeral=yes",
            "MountAPIVFS=yes",
            "ExtensionDirectories=/run/ext/myext",
            "ExtensionImages=/var/lib/ext.raw",
            "MountImages=/img.raw:/mnt",
            "BindLogSockets=yes",
        ]);
        assert_eq!(s.root_directory, Some("/srv/app".to_owned()));
        assert_eq!(s.root_image, Some("/var/lib/app.raw".to_owned()));
        assert_eq!(s.root_image_options, vec!["root:ro"]);
        assert_eq!(s.root_hash, Some("deadbeef".to_owned()));
        assert_eq!(s.root_hash_signature, Some("base64:AAAA".to_owned()));
        assert_eq!(s.root_verity, Some("/var/lib/app.verity".to_owned()));
        assert_eq!(s.root_ephemeral, Some(true));
        assert_eq!(s.mount_api_vfs, Some(true));
        assert_eq!(s.extension_directories, vec!["/run/ext/myext"]);
        assert_eq!(s.extension_images, vec!["/var/lib/ext.raw"]);
        assert_eq!(s.mount_images, vec!["/img.raw:/mnt"]);
        assert_eq!(s.bind_log_sockets, Some(true));
    }

    #[test]
    fn exec_all_new_namespace_directives_together() {
        let s = exec_from_lines(&[
            "PrivateIPC=yes",
            "PrivatePIDs=yes",
            "IPCNamespacePath=/proc/1/ns/ipc",
            "NetworkNamespacePath=/run/netns/isolated",
        ]);
        assert_eq!(s.private_ipc, Some(true));
        assert_eq!(s.private_pids, Some(true));
        assert_eq!(s.ipc_namespace_path, Some("/proc/1/ns/ipc".to_owned()));
        assert_eq!(
            s.network_namespace_path,
            Some("/run/netns/isolated".to_owned())
        );
    }

    #[test]
    fn exec_all_new_security_directives_together() {
        let s = exec_from_lines(&[
            "SecureBits=keep-caps noroot",
            "Personality=x86-64",
            "SELinuxContext=system_u:system_r:init_t:s0",
            "AppArmorProfile=my-profile",
            "SmackProcessLabel=MyLabel",
            "KeyringMode=private",
            "NoExecPaths=/tmp /dev/shm",
            "ExecPaths=/usr/bin /usr/sbin",
            "CoredumpFilter=private-anonymous",
        ]);
        assert_eq!(s.secure_bits, vec!["keep-caps", "noroot"]);
        assert_eq!(s.personality, Some("x86-64".to_owned()));
        assert_eq!(
            s.selinux_context,
            Some("system_u:system_r:init_t:s0".to_owned())
        );
        assert_eq!(s.apparmor_profile, Some("my-profile".to_owned()));
        assert_eq!(s.smack_process_label, Some("MyLabel".to_owned()));
        assert_eq!(s.keyring_mode, Some("private".to_owned()));
        assert_eq!(s.no_exec_paths, vec!["/tmp", "/dev/shm"]);
        assert_eq!(s.exec_paths, vec!["/usr/bin", "/usr/sbin"]);
        assert_eq!(s.coredump_filter, Some("private-anonymous".to_owned()));
    }

    #[test]
    fn exec_all_new_misc_directives_together() {
        let s = exec_from_lines(&[
            "TimerSlackNSec=100000",
            "StandardInputText=hello world",
            "StandardInputData=QUFB",
            "SetLoginEnvironment=yes",
        ]);
        assert_eq!(s.timer_slack_nsec, Some("100000".to_owned()));
        assert_eq!(s.standard_input_text, vec!["hello", "world"]);
        assert_eq!(s.standard_input_data, vec!["QUFB"]);
        assert_eq!(s.set_login_environment, Some(true));
    }

    #[test]
    fn exec_new_directives_coexist_with_existing() {
        let s = exec_from_lines(&[
            "User=nobody",
            "Group=nogroup",
            "PrivateTmp=yes",
            "ProtectSystem=strict",
            "SyslogFacility=daemon",
            "CPUSchedulingPolicy=fifo",
            "RootDirectory=/srv/app",
            "PrivateIPC=yes",
            "SecureBits=keep-caps",
            "TimerSlackNSec=50000",
        ]);
        assert_eq!(s.user, Some("nobody".to_owned()));
        assert_eq!(s.group, Some("nogroup".to_owned()));
        assert!(s.private_tmp);
        assert_eq!(s.syslog_facility, Some("daemon".to_owned()));
        assert_eq!(s.cpu_scheduling_policy, Some("fifo".to_owned()));
        assert_eq!(s.root_directory, Some("/srv/app".to_owned()));
        assert_eq!(s.private_ipc, Some(true));
        assert_eq!(s.secure_bits, vec!["keep-caps"]);
        assert_eq!(s.timer_slack_nsec, Some("50000".to_owned()));
    }

    #[test]
    fn exec_empty_values_reset_to_none() {
        let s = exec_from_lines(&[
            "SyslogFacility=",
            "SyslogLevel=",
            "LogLevelMax=",
            "CPUSchedulingPolicy=",
            "RootDirectory=",
            "Personality=",
            "TimerSlackNSec=",
        ]);
        assert_eq!(s.syslog_facility, None);
        assert_eq!(s.syslog_level, None);
        assert_eq!(s.log_level_max, None);
        assert_eq!(s.cpu_scheduling_policy, None);
        assert_eq!(s.root_directory, None);
        assert_eq!(s.personality, None);
        assert_eq!(s.timer_slack_nsec, None);
    }

    #[test]
    fn exec_list_directives_accumulate_and_reset() {
        // First set values, then add more, then reset
        let s = exec_from_lines(&["CPUAffinity=0 1", "CPUAffinity=", "CPUAffinity=4 5 6"]);
        assert_eq!(s.cpu_affinity, vec!["4", "5", "6"]);
    }

    // ════════════════════════════════════════════════════════════════════
    // Line continuation (backslash at end of line)
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn line_continuation_basic() {
        let content = "\
[Unit]
Description=A very long \\
description value
";
        let parsed = parse_file(content).unwrap();
        let section = parsed.get("[Unit]").unwrap();
        let desc = &section.get("DESCRIPTION").unwrap()[0].1;
        assert_eq!(desc, "A very long description value");
    }

    #[test]
    fn line_continuation_multi_line() {
        let content = "\
[Unit]
Description=first \\
second \\
third
";
        let parsed = parse_file(content).unwrap();
        let section = parsed.get("[Unit]").unwrap();
        let desc = &section.get("DESCRIPTION").unwrap()[0].1;
        assert_eq!(desc, "first second third");
    }

    #[test]
    fn line_continuation_exec_start() {
        let content = "\
[Service]
ExecStart=/usr/bin/foo \\
  --option1=value1 \\
  --option2=value2
";
        let parsed = parse_file(content).unwrap();
        let section = parsed.get("[Service]").unwrap();
        let exec = &section.get("EXECSTART").unwrap()[0].1;
        assert_eq!(exec, "/usr/bin/foo --option1=value1 --option2=value2");
    }

    #[test]
    fn line_continuation_no_backslash_unchanged() {
        let content = "\
[Unit]
Description=normal line
";
        let parsed = parse_file(content).unwrap();
        let section = parsed.get("[Unit]").unwrap();
        let desc = &section.get("DESCRIPTION").unwrap()[0].1;
        assert_eq!(desc, "normal line");
    }

    #[test]
    fn line_continuation_trailing_backslash_at_eof() {
        // Edge case: file ends with a continuation backslash
        let content = "\
[Unit]
Description=trailing \\";
        let parsed = parse_file(content).unwrap();
        let section = parsed.get("[Unit]").unwrap();
        let desc = &section.get("DESCRIPTION").unwrap()[0].1;
        assert_eq!(desc, "trailing");
    }

    #[test]
    fn line_continuation_preserves_separate_keys() {
        let content = "\
[Unit]
Description=hello \\
world
After=foo.service
";
        let parsed = parse_file(content).unwrap();
        let section = parsed.get("[Unit]").unwrap();
        let desc = &section.get("DESCRIPTION").unwrap()[0].1;
        assert_eq!(desc, "hello world");
        let after = &section.get("AFTER").unwrap()[0].1;
        assert_eq!(after, "foo.service");
    }

    #[test]
    fn duplicate_unit_sections_are_merged() {
        let content = "\
[Unit]
Description=test

[Service]
ExecStart=/bin/true

[Unit]
After=bar-template-after.device
";
        let parsed = parse_file(content).unwrap();
        let unit_section = parsed.get("[Unit]").unwrap();
        let desc = &unit_section.get("DESCRIPTION").unwrap()[0].1;
        assert_eq!(desc, "test");
        let after = &unit_section.get("AFTER").unwrap()[0].1;
        assert_eq!(after, "bar-template-after.device");

        let svc_section = parsed.get("[Service]").unwrap();
        assert!(svc_section.contains_key("EXECSTART"));
    }
}
