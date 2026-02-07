/// WGS84 semi-major axis in metres.
const WGS84_A: f64 = 6_378_137.0;
/// WGS84 flattening.
const WGS84_F: f64 = 1.0 / 298.257_223_563;
/// WGS84 first eccentricity squared: e² = 2f - f²
const WGS84_E2: f64 = 2.0 * WGS84_F - WGS84_F * WGS84_F;

/// Convert geodetic (longitude, latitude, altitude) to ECEF XYZ.
///
/// Inputs are in **degrees** and metres.  Returns `[X, Y, Z]` in metres.
pub fn geodetic_to_ecef(lon_deg: f64, lat_deg: f64, alt_m: f64) -> [f64; 3] {
    let lon = lon_deg.to_radians();
    let lat = lat_deg.to_radians();

    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    // Radius of curvature in the prime vertical
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

    let x = (n + alt_m) * cos_lat * cos_lon;
    let y = (n + alt_m) * cos_lat * sin_lon;
    let z = (n * (1.0 - WGS84_E2) + alt_m) * sin_lat;

    [x, y, z]
}

/// Build the 4×4 East-North-Up rotation matrix for a given geodetic point.
///
/// Returns a column-major `[f64; 16]` matrix suitable for `tileset.json`
/// `root.transform`.
pub fn enu_rotation_matrix(lon_deg: f64, lat_deg: f64) -> [f64; 16] {
    let lon = lon_deg.to_radians();
    let lat = lat_deg.to_radians();

    let sin_lon = lon.sin();
    let cos_lon = lon.cos();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();

    // ENU basis vectors expressed in ECEF:
    //   East  = (-sin_lon,          cos_lon,         0       )
    //   North = (-sin_lat*cos_lon, -sin_lat*sin_lon, cos_lat )
    //   Up    = ( cos_lat*cos_lon,  cos_lat*sin_lon, sin_lat )
    //
    // Column-major 4×4 (translation = 0 here; combined in build_root_transform)
    #[rustfmt::skip]
    let m = [
        // column 0 (East)
        -sin_lon,
        cos_lon,
        0.0,
        0.0,
        // column 1 (North)
        -sin_lat * cos_lon,
        -sin_lat * sin_lon,
        cos_lat,
        0.0,
        // column 2 (Up)
        cos_lat * cos_lon,
        cos_lat * sin_lon,
        sin_lat,
        0.0,
        // column 3 (translation placeholder)
        0.0,
        0.0,
        0.0,
        1.0,
    ];

    m
}

/// Combine an ECEF origin and an ENU rotation into a 4×4 root transform.
///
/// Result is a column-major matrix that places the model at `ecef_origin`
/// with local axes oriented to ENU.
pub fn build_root_transform(ecef_origin: [f64; 3], enu_matrix: [f64; 16]) -> [f64; 16] {
    let mut m = enu_matrix;
    // Set the translation column (column 3, rows 0-2)
    m[12] = ecef_origin[0];
    m[13] = ecef_origin[1];
    m[14] = ecef_origin[2];
    m
}

/// Return the 4×4 identity matrix (column-major).
pub fn identity_transform() -> [f64; 16] {
    #[rustfmt::skip]
    let m = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geodetic_to_ecef_equator_prime_meridian() {
        // (0°, 0°, 0m) should give (a, 0, 0)
        let ecef = geodetic_to_ecef(0.0, 0.0, 0.0);
        assert!((ecef[0] - WGS84_A).abs() < 1.0); // X ≈ 6378137
        assert!(ecef[1].abs() < 1.0);               // Y ≈ 0
        assert!(ecef[2].abs() < 1.0);               // Z ≈ 0
    }

    #[test]
    fn geodetic_to_ecef_north_pole() {
        // (0°, 90°, 0m) → X≈0, Y≈0, Z≈b (semi-minor axis)
        let ecef = geodetic_to_ecef(0.0, 90.0, 0.0);
        let b = WGS84_A * (1.0 - WGS84_F); // ≈ 6356752.3
        assert!(ecef[0].abs() < 1.0);
        assert!(ecef[1].abs() < 1.0);
        assert!((ecef[2] - b).abs() < 1.0);
    }

    #[test]
    fn geodetic_to_ecef_known_point() {
        // London (51.5074° N, 0.1278° W, 0m)
        let ecef = geodetic_to_ecef(-0.1278, 51.5074, 0.0);
        // Expected approximately: X≈3978000, Y≈-8700, Z≈4968000
        assert!((ecef[0] - 3_978_000.0).abs() < 1000.0);
        assert!((ecef[1] - (-8700.0)).abs() < 1000.0);
        assert!((ecef[2] - 4_968_000.0).abs() < 1000.0);
    }

    #[test]
    fn geodetic_to_ecef_with_altitude() {
        let ecef_ground = geodetic_to_ecef(0.0, 0.0, 0.0);
        let ecef_high = geodetic_to_ecef(0.0, 0.0, 1000.0);
        // At equator, prime meridian, altitude adds to X only
        assert!((ecef_high[0] - ecef_ground[0] - 1000.0).abs() < 1.0);
    }

    #[test]
    fn enu_matrix_at_equator_prime_meridian() {
        let m = enu_rotation_matrix(0.0, 0.0);
        // East  = (0, 1, 0)  → column 0
        assert!(m[0].abs() < 1e-10);      // -sin(0) = 0
        assert!((m[1] - 1.0).abs() < 1e-10); // cos(0) = 1
        assert!(m[2].abs() < 1e-10);

        // North = (0, 0, 1)  → column 1
        assert!(m[4].abs() < 1e-10);      // -sin(0)*cos(0) = 0
        assert!(m[5].abs() < 1e-10);      // -sin(0)*sin(0) = 0
        assert!((m[6] - 1.0).abs() < 1e-10); // cos(0) = 1

        // Up    = (1, 0, 0)  → column 2
        assert!((m[8] - 1.0).abs() < 1e-10); // cos(0)*cos(0) = 1
        assert!(m[9].abs() < 1e-10);
        assert!(m[10].abs() < 1e-10);
    }

    #[test]
    fn enu_matrix_at_north_pole() {
        let m = enu_rotation_matrix(0.0, 90.0);
        // At the north pole, Up should point along +Z (ECEF)
        // Up = (cos(90)*cos(0), cos(90)*sin(0), sin(90)) = (0, 0, 1)
        assert!(m[8].abs() < 1e-10);
        assert!(m[9].abs() < 1e-10);
        assert!((m[10] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn build_root_transform_sets_translation() {
        let ecef = [100.0, 200.0, 300.0];
        let enu = enu_rotation_matrix(0.0, 0.0);
        let rt = build_root_transform(ecef, enu);
        assert!((rt[12] - 100.0).abs() < 1e-10);
        assert!((rt[13] - 200.0).abs() < 1e-10);
        assert!((rt[14] - 300.0).abs() < 1e-10);
        assert!((rt[15] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn identity_transform_is_correct() {
        let m = identity_transform();
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((m[j * 4 + i] - expected).abs() < 1e-15);
            }
        }
    }
}
