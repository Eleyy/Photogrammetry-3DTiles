pub mod coordinates;
pub mod ecef;
pub mod projection;

use tracing::info;

use crate::config::PipelineConfig;
use crate::error::Result;
use crate::ingestion::IngestionResult;
use crate::types::{BoundingBox, IndexedMesh, MaterialLibrary};

use coordinates::{
    apply_true_north_rotation, apply_unit_scaling, center_meshes, compute_bounding_box,
    swap_y_up_to_z_up, unit_scale_factor,
};
use ecef::{build_root_transform, enu_rotation_matrix, geodetic_to_ecef, identity_transform};

/// Result of the transform stage.
#[derive(Debug)]
pub struct TransformResult {
    pub meshes: Vec<IndexedMesh>,
    pub materials: MaterialLibrary,
    pub root_transform: [f64; 16],
    pub bounds: BoundingBox,
}

/// Run the full transform stage.
pub fn transform(config: &PipelineConfig, ingestion: &IngestionResult) -> Result<TransformResult> {
    // 1. Clone meshes (we modify in-place)
    let mut meshes = ingestion.meshes.clone();
    let materials = ingestion.materials.clone();

    // 2. Unit scaling
    if let Some(units) = config.units {
        let factor = unit_scale_factor(units);
        if (factor - 1.0).abs() > f64::EPSILON {
            info!(units = %units, factor, "Applying unit scaling");
            apply_unit_scaling(&mut meshes, factor);
        }
    }

    // 3. Y-up → Z-up axis swap
    info!("Swapping Y-up to Z-up");
    swap_y_up_to_z_up(&mut meshes);

    // 4. True-north rotation
    let true_north = ingestion
        .georeference
        .as_ref()
        .map(|g| g.true_north)
        .unwrap_or(0.0);
    if true_north.abs() > f64::EPSILON {
        info!(degrees = true_north, "Applying true-north rotation");
        apply_true_north_rotation(&mut meshes, true_north);
    }

    // 5. Center meshes (subtract centroid)
    let centroid = center_meshes(&mut meshes);
    info!(
        cx = centroid[0],
        cy = centroid[1],
        cz = centroid[2],
        "Centered meshes"
    );

    // 6. Compute bounding box
    let bounds = compute_bounding_box(&meshes);

    // 7. Compute root transform
    let root_transform = compute_root_transform(config, ingestion, centroid)?;

    Ok(TransformResult {
        meshes,
        materials,
        root_transform,
        bounds,
    })
}

/// Determine the 4×4 root transform based on georeferencing info.
fn compute_root_transform(
    config: &PipelineConfig,
    ingestion: &IngestionResult,
    centroid: [f64; 3],
) -> Result<[f64; 16]> {
    // Merge georeference from ingestion detection and CLI config
    let georef = ingestion
        .georeference
        .as_ref()
        .or(config.georeference.as_ref());

    let Some(geo) = georef else {
        info!("No georeference -- using identity transform");
        return Ok(identity_transform());
    };

    if geo.epsg == 0 {
        info!("Georeference without EPSG -- using identity transform (local coordinates)");
        return Ok(identity_transform());
    }

    // Project the georeferenced offset (+ centroid) to WGS84
    let origin_easting = geo.easting + centroid[0];
    let origin_northing = geo.northing + centroid[1];
    let origin_elevation = geo.elevation + centroid[2];

    info!(
        epsg = geo.epsg,
        easting = origin_easting,
        northing = origin_northing,
        elevation = origin_elevation,
        "Projecting to WGS84"
    );

    let (lon, lat) = projection::project_to_wgs84(geo.epsg, origin_easting, origin_northing)?;

    info!(lon, lat, "Projected to WGS84");

    let ecef = geodetic_to_ecef(lon, lat, origin_elevation);
    let enu = enu_rotation_matrix(lon, lat);
    let rt = build_root_transform(ecef, enu);

    info!("Computed ECEF root transform");

    Ok(rt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Georeference;
    use crate::ingestion::IngestionStats;

    fn mock_ingestion(meshes: Vec<IndexedMesh>, georef: Option<Georeference>) -> IngestionResult {
        IngestionResult {
            meshes,
            materials: MaterialLibrary::default(),
            georeference: georef,
            stats: IngestionStats {
                total_vertices: 0,
                total_triangles: 0,
                total_meshes: 0,
                has_normals: false,
                has_uvs: false,
                has_colors: false,
                texture_count: 0,
                material_count: 0,
                input_format: "test".into(),
            },
        }
    }

    fn simple_config() -> PipelineConfig {
        PipelineConfig::default()
    }

    #[test]
    fn transform_no_georef_identity() {
        let meshes = vec![IndexedMesh {
            positions: vec![0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        }];
        let ingestion = mock_ingestion(meshes, None);
        let config = simple_config();
        let result = transform(&config, &ingestion).unwrap();

        // Root transform should be identity
        let id = ecef::identity_transform();
        assert_eq!(result.root_transform, id);
    }

    #[test]
    fn transform_with_unit_scaling() {
        let meshes = vec![IndexedMesh {
            positions: vec![1000.0, 0.0, 0.0],
            ..Default::default()
        }];
        let ingestion = mock_ingestion(meshes, None);
        let mut config = simple_config();
        config.units = Some(crate::config::Units::Millimeters);

        let result = transform(&config, &ingestion).unwrap();
        // 1000mm = 1m, then axis swap, then centering (single vertex → stays at 0)
        // After scaling: (1.0, 0.0, 0.0)
        // After Y-up→Z-up: (1.0, 0.0, 0.0) → (1.0, 0.0, -0.0)
        // After centering single vertex: all zero
        assert!(result.meshes[0].positions[0].abs() < 1e-3);
    }

    #[test]
    fn transform_axis_swap_applied() {
        // Y-up triangle: vertex at (1, 2, 3) should become (1, 3, -2) in Z-up
        let meshes = vec![IndexedMesh {
            positions: vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        }];
        let ingestion = mock_ingestion(meshes, None);
        let config = simple_config();
        let result = transform(&config, &ingestion).unwrap();

        // After axis swap: all vertices are (1, 3, -2)
        // After centering: centroid = (1, 3, -2), so all become (0, 0, 0)
        for p in result.meshes[0].positions.chunks_exact(3) {
            assert!(p[0].abs() < 1e-5);
            assert!(p[1].abs() < 1e-5);
            assert!(p[2].abs() < 1e-5);
        }
    }

    #[test]
    fn transform_georef_without_epsg_identity() {
        let meshes = vec![IndexedMesh {
            positions: vec![0.0, 0.0, 0.0],
            ..Default::default()
        }];
        let georef = Georeference {
            epsg: 0,
            easting: 772_598.0,
            northing: 3_575_069.0,
            elevation: 641.0,
            true_north: 0.0,
        };
        let ingestion = mock_ingestion(meshes, Some(georef));
        let config = simple_config();
        let result = transform(&config, &ingestion).unwrap();

        let id = ecef::identity_transform();
        assert_eq!(result.root_transform, id);
    }

    #[test]
    fn transform_georef_with_epsg_produces_ecef() {
        let meshes = vec![IndexedMesh {
            positions: vec![0.0, 0.0, 0.0],
            ..Default::default()
        }];
        let georef = Georeference {
            epsg: 32636,
            easting: 500_000.0,
            northing: 0.0,
            elevation: 0.0,
            true_north: 0.0,
        };
        let ingestion = mock_ingestion(meshes, Some(georef));
        let config = simple_config();
        let result = transform(&config, &ingestion).unwrap();

        // Root transform should NOT be identity -- it should have ECEF translation
        let id = ecef::identity_transform();
        assert_ne!(result.root_transform, id);

        // Translation column should be near the ECEF of (33°, 0°, 0m)
        let tx = result.root_transform[12];
        let ty = result.root_transform[13];
        let tz = result.root_transform[14];
        // ECEF of equator at 33°E should have large X/Y, small Z
        assert!(tx > 5_000_000.0);
        assert!(ty > 3_000_000.0);
        assert!(tz.abs() < 10_000.0);
    }

    #[test]
    fn transform_bounding_box_computed() {
        let meshes = vec![IndexedMesh {
            positions: vec![
                -1.0, 0.0, 0.0, // Y-up: (-1, 0, 0)
                1.0, 2.0, 3.0, // Y-up: (1, 2, 3)
                0.0, 1.0, 0.0, // Y-up: (0, 1, 0)
            ],
            indices: vec![0, 1, 2],
            ..Default::default()
        }];
        let ingestion = mock_ingestion(meshes, None);
        let config = simple_config();
        let result = transform(&config, &ingestion).unwrap();

        // After transform, bounds should be non-degenerate
        let diag = result.bounds.diagonal();
        assert!(diag > 0.0);
    }
}
