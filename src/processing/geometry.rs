use crate::math::{AABB, Vec3};
use crate::{GEOMETRY_OFFSET_UNITS, LightCluster, TARGET_MATERIAL, UV_SEARCH_DIST, utils};
use log::{debug, info, warn};
use vmf_forge::VmfFile;
use vmf_forge::prelude::{Entity, Solid};

#[derive(Debug, Clone)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
    pub u_axis: String,
    pub v_axis: String,
    pub material: String,
}

impl Plane {
    pub fn new(normal: Vec3, dist: f32) -> Self {
        Self {
            normal,
            dist,
            u_axis: String::new(),
            v_axis: String::new(),
            material: String::from("default"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConvexBrush {
    pub id: u64,
    pub planes: Vec<Plane>,
    pub _bounds: AABB,
}

impl ConvexBrush {
    /// Converts a VMF Solid into a mathematical ConvexBrush
    pub fn from_vmf_solid(solid: &Solid) -> Option<Self> {
        let mut planes = Vec::with_capacity(solid.sides.len());
        let mut aabb = AABB::new();
        let mut valid_points_found = false;

        // Check if this is a displacement brush
        // In Source, if a brush has a displacement face, only that face "exists" for physics/vis usually.
        let is_displacement = solid.sides.iter().any(|s| s.dispinfo.is_some());
        if is_displacement {
            return None
        }

        for side in &solid.sides {
            // Parse 3 points of the plane
            let points = match utils::text::parse_plane_points(&side.plane) {
                Some(pts) => pts,
                None => {
                    warn!("Solid ID {}: Malformed plane definition found. Side has less than 3 points. Side plane: '{}'", solid.id, side.plane);
                    continue; // Broken plane definition
                }
            };

            // Approximately, using plane points. For a precise AABB,
            // one would need to find plane intersections, but for VMF, plane points
            // usually lie on the brush corners, so this is okay
            aabb.extend(points[0]);
            aabb.extend(points[1]);
            aabb.extend(points[2]);
            valid_points_found = true;

            // Calculate the plane normal
            let n = utils::text::calc_face_normal(points) * -1.0; // todo haha
            let d = -n.dot(points[0]);

            planes.push(Plane {
                normal: n,
                dist: d,
                u_axis: side.u_axis.clone(),
                v_axis: side.v_axis.clone(),
                material: side.material.clone(),
            });
        }

        if planes.is_empty() || !valid_points_found {
            warn!("Solid ID {} was skipped because it contains no valid planes.", solid.id);
            return None;
        }

        debug!("Created ConvexBrush for solid ID {} with {} planes. AABB: min={:?}, max={:?}", solid.id, planes.len(), aabb.min, aabb.max);
        Some(ConvexBrush {
            id: solid.id,
            planes,
            _bounds: aabb,
        })
    }
}

pub fn get_entity_aabb(ent: &Entity) -> Option<AABB> {
    let solids = ent.solids.as_ref()?;
    if solids.is_empty() { return None; }

    // Re-use logic from ConvexBrush parsing but for AABB
    let mut aabb = AABB::new();
    let mut found = false;

    for solid in solids {
        for side in &solid.sides {
            if let Some(points) = utils::text::parse_plane_points(&side.plane) {
                for p in points {
                    aabb.extend(p);
                }
                found = true;
            }
        }
    }

    if !found { return None; }
    Some(aabb)
}

// todo!!!!!!!!!!! DRY
pub fn get_solid_aabb(solid: &Solid) -> Option<AABB> {
    // Re-use logic from ConvexBrush parsing but for AABB
    let mut aabb = AABB::new();
    let mut found = false;

    for side in solid.sides.iter() {
        if let Some(points) = utils::text::parse_plane_points(&side.plane) {
            for p in points {
                aabb.extend(p);
            }
            found = true;
        }
    }

    if !found { return None; }
    Some(aabb)
}

/// Builds the collision world from VMF solids and func_details
pub fn build_collision_world(vmf: &VmfFile) -> Vec<ConvexBrush> {
    debug!("Building collision world...");
    let mut brushes = Vec::new();

    // World Solids (worldspawn)
    debug!("Processing {} world solids...", vmf.world.solids.len());
    for solid in &vmf.world.solids {
        if let Some(brush) = ConvexBrush::from_vmf_solid(solid) {
            brushes.push(brush);
        }
    }

    // Func Detail
    for ent in vmf.entities.iter() {
        let classname = ent.classname().unwrap_or("");
        if let Some(should_skip) = ent.get("pbr_geometry_ignore") {  // todo
            if should_skip != "0" { continue }
        }

        if classname == "func_detail" { // ? i think we should ignore any dynamic ents
            debug!("Found collidable entity: class='{}', targetname='{}'", classname, ent.targetname().unwrap_or("N/A"));
            if let Some(solids) = &ent.solids {
                for solid in solids {
                    if let Some(brush) = ConvexBrush::from_vmf_solid(solid) {
                        brushes.push(brush);
                    }
                }
            }
        }
    }

    info!("Built collision world with {} brushes.", brushes.len());
    brushes
}

/// Offsets solid geometry to prevent z-fighting and copies UVs from parent surfaces
pub fn apply_offsets_and_uv_fixes(
    clusters: &[LightCluster],
    map_name: &str,
    world_brushes: &[ConvexBrush],
) {
    for cluster in clusters {
        // eg. "maps/sp_a2_triple_laser/surface_0_solid_0"
        let patch_material_str = format!("maps/{}/{}", map_name, cluster.surface_material);

        for solid_arc in &cluster.solids {
            let mut solid = solid_arc.write().unwrap();
            let normal = solid.surface_normal;

            // Calc geometry offset
            let max_axis = normal.0.abs().max(normal.1.abs()).max(normal.2.abs());
            let offset = normal * (GEOMETRY_OFFSET_UNITS * max_axis);

            // Raycast for find 'parent' and fix UV
            // TODO: or parse all solids to find it, OR make it in creating GgxSolid!
            // BECAUSE for now it's not a very stable and efficient solution.
            let target_pts = solid.sides.iter()
                .find(|side| side.material.eq_ignore_ascii_case(TARGET_MATERIAL))
                .and_then(|side| crate::text::parse_plane_points(&side.plane));

            let start_pos = if let Some(pts) = target_pts {
                let p0 = pts[0];
                let k = (p0 - solid.bound.center).dot(normal);
                let point_on_plane = solid.bound.center + (normal * k);

                point_on_plane
            } else {
                solid.bound.center
            } + (normal * 5.0);

            debug!("  [UV Fix] Casting ray from {:?} dir {:?} (dist: {})", start_pos, normal, UV_SEARCH_DIST);

            let ray_dir = normal * -1.0;
            let parent_uv = crate::tracer::trace_ray_closest(start_pos, ray_dir, UV_SEARCH_DIST, &world_brushes)
                .inspect(|hit| debug!("    -> x {:.2} (brush id: {}). Copying UVs.", hit.t, hit.id))
                .map(|hit| {
                    (hit.u_axis, hit.v_axis)
                });

            if parent_uv.is_none() {
                debug!("    -> No parent surface found within range.");
            }

            let mut material_updated = false;

            // Modify solid sides
            for side in &mut solid.sides {
                side.plane = crate::text::apply_offset_to_plane(&side.plane, offset);

                // update material
                if side.material.eq_ignore_ascii_case(TARGET_MATERIAL) {
                    if let Some((u, v)) = parent_uv {
                        side.u_axis = u.to_string();
                        side.v_axis = v.to_string();
                    } else {
                        warn!("No parent_uv found for cluster: {}", cluster.name);
                    }

                    side.material = patch_material_str.clone();
                    material_updated = true;
                }
            }

            if !material_updated {
                // todo: skip and remove cluster?
                warn!("The cluster {} (ggx_surface: {}) has no {} texture. PBS will be skipped!", cluster.name, cluster.ggx_surface_name, TARGET_MATERIAL);
            }
        }
    }
}
