use log::trace;

use crate::units::{
    ParsedCommonConfig, ParsedFile, ParsedTargetConfig, ParsingErrorReason, parse_install_section,
    parse_unit_section,
};
use std::path::PathBuf;

pub fn parse_target(
    parsed_file: ParsedFile,
    path: &PathBuf,
) -> Result<ParsedTargetConfig, ParsingErrorReason> {
    let mut install_config = None;
    let mut unit_config = None;

    for (name, section) in parsed_file {
        match name.as_str() {
            "[Unit]" => {
                unit_config = Some(parse_unit_section(section)?);
            }
            "[Install]" => {
                install_config = Some(parse_install_section(section)?);
            }
            _ if name.starts_with("[X-") || name.starts_with("[x-") => {
                trace!(
                    "Silently ignoring vendor extension section in target unit {path:?}: {name}"
                );
            }
            _ => {
                trace!("Ignoring unknown section in target unit {path:?}: {name}");
            }
        }
    }

    Ok(ParsedTargetConfig {
        common: ParsedCommonConfig {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            unit: unit_config.unwrap_or_else(Default::default),
            install: install_config.unwrap_or_else(Default::default),
            fragment_path: Some(path.clone()),
        },
    })
}
