use crate::math::{cross, dot, normalize, sub};
use crate::types::{LightCluster, LightType};
use log::{warn};
use std::fs::File;
use std::io::Write;

use std::path::Path;

pub const LUT_WIDTH: usize = 8;
pub const LUT_HEIGHT: usize = 8;

pub fn generate_vtf(cluster: &LightCluster, output_path: &Path) -> anyhow::Result<()> {
    let num_lights = cluster.lights.len();

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if num_lights > LUT_WIDTH {
        warn!(
            "Cluster '{}': More than {} lights provided ({}). Truncating.",
            cluster.name, LUT_WIDTH, num_lights
        );
    }

    // RGBA F32 buffer
    let mut rgba_pixels = vec![(0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32); LUT_WIDTH * LUT_HEIGHT];

    for (i, (light, _score)) in cluster.lights.iter().take(LUT_WIDTH).enumerate() {
        let mut dir = [0.0, 0.0, 0.0];
        let mut param1 = 0.0;
        let mut param2 = 0.0;
        let mut extra_param = 0.0;
        let type_id;

        match &light.light_type {
            LightType::Point => type_id = 0.0,
            LightType::Spot {
                direction,
                inner_angle,
                outer_angle,
                exponent,
            } => {
                type_id = 1.0;
                dir = *direction;
                param1 = inner_angle.to_radians().cos();
                param2 = outer_angle.to_radians().cos();
                extra_param = *exponent;
            }
            LightType::Rect {
                direction,
                width: w,
                height: h,
                bidirectional,
            } => {
                type_id = 2.0;
                dir = *direction;
                param1 = *w;
                param2 = *h;
                if *bidirectional {
                    extra_param = 1.0;
                }
            }
        }

        // WRITE TO TEXTURE ROWS
        rgba_pixels[0 * LUT_WIDTH + i] = (light.pos[0], light.pos[1], light.pos[2], type_id);
        rgba_pixels[1 * LUT_WIDTH + i] = (light.color[0], light.color[1], light.color[2], light.intensity);
        rgba_pixels[2 * LUT_WIDTH + i] = (dir[0], dir[1], dir[2], param1);
        rgba_pixels[3 * LUT_WIDTH + i] = (light.range, light.attenuation_k, param2, extra_param);

        for row in 4..=7 {
            rgba_pixels[row * LUT_WIDTH + i] = (0.0, 0.0, 0.0, 0.0);
        }

        for (b_idx, b) in light.blockers.iter()
            .enumerate()
            .filter_map(|(i, opt)| opt.as_ref().map(|val| (i, val)))
        {
            let base_row = 4 + (b_idx * 2);
            let is_fizzler = b.flag == 2;

            // Blocker Params: Size
            if is_fizzler {
                rgba_pixels[base_row * LUT_WIDTH + i] = (b.width, b.depth, b.height, b.flag as f32);
            } else {
                rgba_pixels[base_row * LUT_WIDTH + i] = (b.width, b.height, b.depth, b.flag as f32);
            }

            // Blocker Offset
            let blocker_world_pos = b.pos.unwrap_or(light.pos);
            let diff = sub(blocker_world_pos, light.pos);

            if is_fizzler {
                // Project offset to light local space for Fizzlers
                let light_dir = normalize(dir);
                let up_base = if light_dir[2].abs() > 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
                let right = normalize(cross(light_dir, up_base));
                let up = cross(right, light_dir);

                let off_x = dot(diff, right);
                let off_y = dot(diff, up);
                let off_z = dot(diff, light_dir);
                rgba_pixels[(base_row + 1) * LUT_WIDTH + i] = (off_x, off_y, off_z, 0.0);
            } else {
                // World space offset
                rgba_pixels[(base_row + 1) * LUT_WIDTH + i] = (diff[0], diff[1], diff[2], 0.0);
            }
        }
    }
    // -----------------------------------------------------

    let mut raw_data = Vec::with_capacity(rgba_pixels.len() * 4);
    for pixel in rgba_pixels {
        raw_data.push(pixel.0); // R
        raw_data.push(pixel.1); // G
        raw_data.push(pixel.2); // B
        raw_data.push(pixel.3); // A
    }

    // Write VTF directly
    let vtf_path = output_path.with_extension("vtf");
    let params = crate::vtf_writer::VtfParams {
        width: LUT_WIDTH as u16,
        height: LUT_HEIGHT as u16,
    };

    crate::vtf_writer::write_rgba32f_vtf(&vtf_path, params, &raw_data)
}


/// Generates a Patch VMT that includes the base PBR shader and inserts the generated LUT
pub fn generate_vmt(vmt_path: &Path, texture_rel_path: &str, base_material: Option<&str>, initial_c4: [f32; 4], surface_id: u64) -> anyhow::Result<()> {
    if let Some(parent) = vmt_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = File::create(vmt_path)?;
    let include_path = base_material.ok_or_else(|| {
        anyhow::anyhow!(
            "Missing 'template_material' in entity properties for {:?} (id: {}). This is required!",
            vmt_path.file_stem().unwrap_or_default(), surface_id
        )
    })?;

    writeln!(file, "patch")?;
    writeln!(file, "{{")?;
    writeln!(file, "\tinclude \"materials/{}.vmt\"", include_path)?;
    writeln!(file, "\treplace")?;
    writeln!(file, "\t{{")?;

    // Normalize path separators to forward slashes for Source Engine
    let clean_path = texture_rel_path.replace('\\', "/");
    writeln!(file, "\t\t$texture1 \"{}\"", clean_path)?;


    // Write $c4 vector based on light initial state
    writeln!(file, "\t\t$c4_x {:.2}", initial_c4[0])?;
    writeln!(file, "\t\t$c4_y {:.2}", initial_c4[1])?;
    writeln!(file, "\t\t$c4_z {:.2}", initial_c4[2])?;
    writeln!(file, "\t\t$c4_w {:.2}", initial_c4[3])?;

    writeln!(file, "\t}}")?;
    writeln!(file, "}}")?;

    Ok(())
}
