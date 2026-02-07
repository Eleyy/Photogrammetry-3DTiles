use std::fs;
use std::path::Path;

use tracing::{debug, warn};

use crate::config::{Georeference, PipelineConfig};
use crate::error::{PhotoTilerError, Result};

/// Detect georeferencing from CLI overrides, metadata files, or project files.
///
/// Priority: CLI override > metadata.xml > offset.xyz + .prj > none
pub fn detect_georeference(config: &PipelineConfig) -> Result<Option<Georeference>> {
    // 1. CLI override (already resolved in config)
    if config.georeference.is_some() {
        debug!("Using CLI-provided georeference");
        return Ok(config.georeference.clone());
    }

    let input_dir = config
        .input
        .parent()
        .unwrap_or_else(|| Path::new("."));

    // 2. metadata.xml
    let xml_path = config
        .metadata_xml
        .clone()
        .or_else(|| {
            let candidate = input_dir.join("metadata.xml");
            candidate.exists().then_some(candidate)
        });
    if let Some(ref path) = xml_path {
        debug!(path = %path.display(), "Checking metadata.xml");
        if let Some(georef) = parse_metadata_xml(path)? {
            return Ok(Some(georef));
        }
    }

    // 3. offset.xyz + optional .prj
    let offset_path = config
        .offset_file
        .clone()
        .or_else(|| {
            let candidate = input_dir.join("offset.xyz");
            candidate.exists().then_some(candidate)
        });
    if let Some(ref path) = offset_path {
        debug!(path = %path.display(), "Checking offset.xyz");
        let (easting, northing, elevation) = parse_offset_xyz(path)?;
        let epsg = find_prj_epsg(input_dir).unwrap_or(0);
        return Ok(Some(Georeference {
            epsg,
            easting,
            northing,
            elevation,
            true_north: 0.0,
        }));
    }

    debug!("No georeference detected");
    Ok(None)
}

/// Parse an `offset.xyz` file containing `easting northing elevation`.
pub fn parse_offset_xyz(path: &Path) -> Result<(f64, f64, f64)> {
    let content = fs::read_to_string(path).map_err(|e| {
        PhotoTilerError::Georeference(format!("Failed to read offset.xyz: {e}"))
    })?;

    let values: Vec<f64> = content
        .split_whitespace()
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    if values.len() < 3 {
        return Err(PhotoTilerError::Georeference(format!(
            "offset.xyz must contain at least 3 numeric values, found {}",
            values.len()
        )));
    }

    Ok((values[0], values[1], values[2]))
}

/// Extract EPSG code and offset from Agisoft/DJI metadata XML.
pub fn parse_metadata_xml(path: &Path) -> Result<Option<Georeference>> {
    let content = fs::read_to_string(path).map_err(|e| {
        PhotoTilerError::Georeference(format!("Failed to read metadata.xml: {e}"))
    })?;

    // Try to extract EPSG from the XML content
    let epsg = extract_epsg_from_string(&content);

    // Look for offset/transform values in common XML patterns
    // Agisoft: <transform> or <offset x="..." y="..." z="...">
    // For now, we just extract the EPSG if present
    if let Some(epsg) = epsg {
        debug!(epsg, "Found EPSG in metadata.xml");
        return Ok(Some(Georeference {
            epsg,
            easting: 0.0,
            northing: 0.0,
            elevation: 0.0,
            true_north: 0.0,
        }));
    }

    warn!("metadata.xml found but no EPSG code detected");
    Ok(None)
}

/// Scan a directory for `.prj` files and extract an EPSG code.
pub fn find_prj_epsg(dir: &Path) -> Result<u32> {
    let entries = fs::read_dir(dir).map_err(|e| {
        PhotoTilerError::Georeference(format!("Failed to read directory {}: {e}", dir.display()))
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("prj") {
            debug!(path = %path.display(), "Found .prj file");
            let content = fs::read_to_string(&path).map_err(|e| {
                PhotoTilerError::Georeference(format!("Failed to read .prj file: {e}"))
            })?;
            if let Some(epsg) = extract_epsg_from_string(&content) {
                return Ok(epsg);
            }
        }
    }

    Err(PhotoTilerError::Georeference(
        "No .prj file with EPSG code found".into(),
    ))
}

/// Extract an EPSG code from a string.
///
/// Matches patterns:
/// - `EPSG:12345`
/// - `EPSG::12345`
/// - WKT `AUTHORITY["EPSG","12345"]`
pub fn extract_epsg_from_string(content: &str) -> Option<u32> {
    // Pattern 1: EPSG:12345 or EPSG::12345
    if let Some(pos) = content.find("EPSG:") {
        let after = &content[pos + 5..];
        // Skip optional second colon
        let after = after.strip_prefix(':').unwrap_or(after);
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(epsg) = num_str.parse::<u32>() {
            if epsg > 0 {
                return Some(epsg);
            }
        }
    }

    // Pattern 2: AUTHORITY["EPSG","12345"]
    if let Some(pos) = content.find("AUTHORITY[\"EPSG\"") {
        let after = &content[pos..];
        // Find the second quoted number
        if let Some(comma_pos) = after.find(',') {
            let after_comma = &after[comma_pos + 1..];
            let num_str: String = after_comma
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(epsg) = num_str.parse::<u32>() {
                if epsg > 0 {
                    return Some(epsg);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_valid_offset_xyz() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("offset.xyz");
        fs::write(&path, "772598.000 3575069.000 641.000").unwrap();

        let (e, n, el) = parse_offset_xyz(&path).unwrap();
        assert!((e - 772598.0).abs() < f64::EPSILON);
        assert!((n - 3575069.0).abs() < f64::EPSILON);
        assert!((el - 641.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_offset_xyz_with_newlines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("offset.xyz");
        fs::write(&path, "772598.000\n3575069.000\n641.000\n").unwrap();

        let (e, n, el) = parse_offset_xyz(&path).unwrap();
        assert!((e - 772598.0).abs() < f64::EPSILON);
        assert!((n - 3575069.0).abs() < f64::EPSILON);
        assert!((el - 641.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_invalid_offset_xyz() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("offset.xyz");
        fs::write(&path, "772598.000 abc").unwrap();

        let err = parse_offset_xyz(&path).unwrap_err();
        assert!(err.to_string().contains("at least 3 numeric values"));
    }

    #[test]
    fn extract_epsg_single_colon() {
        assert_eq!(extract_epsg_from_string("EPSG:32636"), Some(32636));
    }

    #[test]
    fn extract_epsg_double_colon() {
        assert_eq!(extract_epsg_from_string("EPSG::32636"), Some(32636));
    }

    #[test]
    fn extract_epsg_wkt_authority() {
        let wkt = r#"PROJCS["WGS 84 / UTM zone 36N",AUTHORITY["EPSG","32636"]]"#;
        assert_eq!(extract_epsg_from_string(wkt), Some(32636));
    }

    #[test]
    fn extract_epsg_none() {
        assert_eq!(extract_epsg_from_string("no epsg here"), None);
    }

    #[test]
    fn detect_from_offset_and_prj() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("model.obj");
        fs::write(&input, "").unwrap();
        fs::write(dir.path().join("offset.xyz"), "100.0 200.0 50.0").unwrap();
        fs::write(dir.path().join("model.prj"), "EPSG:32636").unwrap();

        let config = PipelineConfig {
            input,
            ..Default::default()
        };

        let georef = detect_georeference(&config).unwrap().unwrap();
        assert_eq!(georef.epsg, 32636);
        assert!((georef.easting - 100.0).abs() < f64::EPSILON);
        assert!((georef.northing - 200.0).abs() < f64::EPSILON);
        assert!((georef.elevation - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn detect_cli_override_takes_priority() {
        let config = PipelineConfig {
            georeference: Some(Georeference {
                epsg: 4326,
                easting: 1.0,
                northing: 2.0,
                elevation: 3.0,
                true_north: 0.0,
            }),
            ..Default::default()
        };

        let georef = detect_georeference(&config).unwrap().unwrap();
        assert_eq!(georef.epsg, 4326);
    }

    #[test]
    fn detect_returns_none_when_no_files() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("model.obj");
        fs::write(&input, "").unwrap();

        let config = PipelineConfig {
            input,
            ..Default::default()
        };

        assert!(detect_georeference(&config).unwrap().is_none());
    }
}
