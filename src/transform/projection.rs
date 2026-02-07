use crate::error::{PhotoTilerError, Result};

/// Project an (easting, northing) pair from the given EPSG CRS to WGS84.
///
/// Returns `(longitude, latitude)` in degrees.
pub fn project_to_wgs84(epsg: u32, easting: f64, northing: f64) -> Result<(f64, f64)> {
    let from = format!("EPSG:{epsg}");
    let proj = proj::Proj::new_known_crs(&from, "EPSG:4326", None).map_err(|e| {
        PhotoTilerError::Transform(format!(
            "Failed to create projection from {from} to WGS84: {e}"
        ))
    })?;

    let (lon, lat) = proj
        .convert((easting, northing))
        .map_err(|e| PhotoTilerError::Transform(format!("Projection failed: {e}")))?;

    Ok((lon, lat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utm_zone_36n_to_wgs84() {
        // EPSG:32636 = UTM zone 36N
        // UTM zone 36N central meridian = 33°E
        // (500000, 0) in UTM 36N should map to approximately (33°, 0°)
        let (lon, lat) = project_to_wgs84(32636, 500_000.0, 0.0).unwrap();
        assert!(
            (lon - 33.0).abs() < 0.01,
            "longitude {lon} should be near 33.0"
        );
        assert!(lat.abs() < 0.01, "latitude {lat} should be near 0.0");
    }

    #[test]
    fn utm_zone_36n_known_offset() {
        // The actual test data has offset.xyz: 772598.000 3575069.000 641.000
        // This is a point in UTM zone 36N (EPSG:32636)
        let (lon, lat) = project_to_wgs84(32636, 772_598.0, 3_575_069.0).unwrap();
        // Should be somewhere in the eastern Mediterranean / Middle East
        assert!(lon > 30.0 && lon < 40.0, "longitude {lon} out of range");
        assert!(lat > 30.0 && lat < 35.0, "latitude {lat} out of range");
    }

    #[test]
    fn invalid_epsg_returns_error() {
        let result = project_to_wgs84(99999, 0.0, 0.0);
        assert!(result.is_err());
    }
}
