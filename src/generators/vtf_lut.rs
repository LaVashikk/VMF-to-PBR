use std::path::Path;

use crate::math::Vec3;
use crate::types::LightType;
use crate::{types::LightCluster, vmt_helper::VmtPbrParams};

use crate::constants::{LUT_WIDTH, LUT_HEIGHT};

pub fn generate(cluster: &LightCluster, output_path: &Path, params: &VmtPbrParams) -> anyhow::Result<()> {
    let num_lights = cluster.lights.len();

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if num_lights > LUT_WIDTH {
        log::warn!(
            "Cluster '{}': More than {} lights provided ({}). Truncating.",
            cluster.name, LUT_WIDTH, num_lights
        );
    }

    // RGBA F32 buffer
    let mut rgba_pixels = vec![(0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32); LUT_WIDTH * LUT_HEIGHT];

    for (i, (light, _score)) in cluster.lights.iter().take(LUT_WIDTH).enumerate() {
        let mut dir = Vec3::ZERO;
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
            let diff = blocker_world_pos - light.pos;

            if is_fizzler {
                // Project offset to light local space for Fizzlers
                let light_dir = dir.normalize();
                let up_base = if light_dir[2].abs() > 0.99 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 0.0, 1.0) };
                let right = light_dir.cross(up_base).normalize();
                let up = right.cross(light_dir).normalize();

                let off_x = diff.dot(right);
                let off_y = diff.dot(up);
                let off_z = diff.dot(light_dir);
                rgba_pixels[(base_row + 1) * LUT_WIDTH + i] = (off_x, off_y, off_z, 0.0);
            } else {
                // World space offset
                rgba_pixels[(base_row + 1) * LUT_WIDTH + i] = (diff[0], diff[1], diff[2], 0.0);
            }
        }
    }

    // --- WRITE GLOBAL PARAMS (ROW 8) ---
    let params_row = 8;

    // c0_data: x = RoughnessBias, y = DielectricF0, z = GlobalIntensity, w = UV_Scale
    rgba_pixels[params_row * LUT_WIDTH + 0] = (
        params.roughness_bias,
        params.dielectric_f0,
        params.global_intensity,
        params.uv_scale,
    );

    // c1_data: Tintable (R, G, B, A)
    rgba_pixels[params_row * LUT_WIDTH + 1] = (
        params.albedo_tint[0],
        params.albedo_tint[1],
        params.albedo_tint[2],
        params.albedo_tint[3],
    );

    // c2_data: x = FadeStart, y = FadeEnd, z = NumLights, w = UseCubemap
    let use_cubemap_f32 = if params.use_cubemap { 1.0 } else { 0.0 };
    rgba_pixels[params_row * LUT_WIDTH + 2] = (
        params.fade_start,
        params.fade_end,
        params.num_lights,
        use_cubemap_f32,
    );

    // c3_data: x = NormalStrength, y = ReflectionStrength, z = AO_Strength, w = MetalnessScale
    rgba_pixels[params_row * LUT_WIDTH + 3] = (
        params.normal_scale,
        params.reflection_scale,
        params.ao_scale,
        params.metalness_scale,
    );

    // --- WRITE PCC DATA (ROW 15) ---
    if let Some(pcc) = &cluster.pcc_volume {
        let row = 15;
        // World Space Box Min
        rgba_pixels[row * LUT_WIDTH + 0] = (pcc.ws_min[0], pcc.ws_min[1], pcc.ws_min[2], 1.0);
        // World Space Box Max
        rgba_pixels[row * LUT_WIDTH + 1] = (pcc.ws_max[0], pcc.ws_max[1], pcc.ws_max[2], 1.0);
        // Pixel 2: Cubemap Origin
        rgba_pixels[row * LUT_WIDTH + 2] = (pcc.cubemap_pos[0], pcc.cubemap_pos[1], pcc.cubemap_pos[2], 1.0);
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
