/// Axis-aligned bounding box in 3-D.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl BoundingBox {
    /// Centre point of the box.
    pub fn center(&self) -> [f64; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }

    /// Half-extents along each axis.
    pub fn half_extents(&self) -> [f64; 3] {
        [
            (self.max[0] - self.min[0]) * 0.5,
            (self.max[1] - self.min[1]) * 0.5,
            (self.max[2] - self.min[2]) * 0.5,
        ]
    }

    /// Length of the space diagonal.
    pub fn diagonal(&self) -> f64 {
        let dx = self.max[0] - self.min[0];
        let dy = self.max[1] - self.min[1];
        let dz = self.max[2] - self.min[2];
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Whether a point lies inside (or on the boundary of) the box.
    pub fn contains_point(&self, p: [f64; 3]) -> bool {
        p[0] >= self.min[0]
            && p[0] <= self.max[0]
            && p[1] >= self.min[1]
            && p[1] <= self.max[1]
            && p[2] >= self.min[2]
            && p[2] <= self.max[2]
    }

    /// Return the smallest box that contains both `self` and `other`.
    pub fn merge(&self, other: &BoundingBox) -> BoundingBox {
        BoundingBox {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }
}

/// Binary GLB payload for a single tile.
#[derive(Debug, Clone)]
pub struct TileContent {
    pub glb_data: Vec<u8>,
    pub uri: String,
}

/// Octree hierarchy node.
#[derive(Debug, Clone)]
pub struct TileNode {
    /// Address string: "root", "0", "0_1", "0_1_3", etc.
    pub address: String,
    pub level: u32,
    pub bounds: BoundingBox,
    pub geometric_error: f64,
    pub content: Option<TileContent>,
    pub children: Vec<TileNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> BoundingBox {
        BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn bounding_box_center() {
        let bb = unit_box();
        let c = bb.center();
        assert!((c[0] - 0.5).abs() < f64::EPSILON);
        assert!((c[1] - 0.5).abs() < f64::EPSILON);
        assert!((c[2] - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn bounding_box_half_extents() {
        let bb = unit_box();
        let he = bb.half_extents();
        assert!((he[0] - 0.5).abs() < f64::EPSILON);
        assert!((he[1] - 0.5).abs() < f64::EPSILON);
        assert!((he[2] - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn bounding_box_diagonal() {
        let bb = unit_box();
        let expected = 3.0_f64.sqrt();
        assert!((bb.diagonal() - expected).abs() < 1e-10);
    }

    #[test]
    fn bounding_box_contains_point() {
        let bb = unit_box();
        assert!(bb.contains_point([0.5, 0.5, 0.5]));
        assert!(bb.contains_point([0.0, 0.0, 0.0])); // boundary
        assert!(bb.contains_point([1.0, 1.0, 1.0])); // boundary
        assert!(!bb.contains_point([1.1, 0.5, 0.5]));
        assert!(!bb.contains_point([-0.1, 0.5, 0.5]));
    }

    #[test]
    fn bounding_box_merge() {
        let a = unit_box();
        let b = BoundingBox {
            min: [-1.0, -1.0, -1.0],
            max: [0.5, 0.5, 0.5],
        };
        let merged = a.merge(&b);
        assert_eq!(merged.min, [-1.0, -1.0, -1.0]);
        assert_eq!(merged.max, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn tile_node_construction() {
        let node = TileNode {
            address: "root".into(),
            level: 0,
            bounds: unit_box(),
            geometric_error: 100.0,
            content: None,
            children: vec![TileNode {
                address: "0".into(),
                level: 1,
                bounds: BoundingBox {
                    min: [0.0, 0.0, 0.0],
                    max: [0.5, 0.5, 0.5],
                },
                geometric_error: 50.0,
                content: Some(TileContent {
                    glb_data: vec![0x67, 0x6C, 0x54, 0x46],
                    uri: "tiles/0/tile.glb".into(),
                }),
                children: vec![],
            }],
        };

        assert_eq!(node.address, "root");
        assert_eq!(node.level, 0);
        assert!(node.content.is_none());
        assert_eq!(node.children.len(), 1);
        assert_eq!(node.children[0].address, "0");
        assert!(node.children[0].content.is_some());
    }
}
