use source_fs::{DummyVpk, FileSystem, FileSystemOptions, P2GameInfo};
use crate::math::Vec3;
use crate::types::{LightCluster, LightType};
use anyhow::{Context, bail};
use std::fs::File;
use std::io::Write;

use std::path::Path;

pub const LUT_WIDTH: usize = 8;
pub const LUT_HEIGHT: usize = 16;

pub fn generate_vtf(cluster: &LightCluster, output_path: &Path, params: VmtParams) -> anyhow::Result<()> {
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

#[derive(Debug, Clone)]
pub struct VmtParams {
    pub use_cubemap: bool,
    pub reflection_scale: f32,
    pub metalness_scale: f32,
    pub roughness_bias: f32,
    pub num_lights: f32,
    pub uv_scale: f32,
    pub ao_scale: f32,
    pub global_intensity: f32,
    pub dielectric_f0: f32,
    pub normal_scale: f32,
    pub fade_start: f32,
    pub fade_end: f32,
    pub albedo_tint: [f32; 4],
}

impl Default for VmtParams {
    fn default() -> Self {
        Self {
            use_cubemap: false,
            reflection_scale: 1.0,
            metalness_scale: 1.0,
            roughness_bias: 1.0,
            num_lights: 0.0,
            uv_scale: 1.0,
            ao_scale: 1.0,
            global_intensity: 1.0,
            dielectric_f0: 0.04,
            normal_scale: 1.0,
            fade_start: 1024.0,
            fade_end: 2048.0,
            albedo_tint: [1.0, 1.0, 1.0, 1.0],
        }
    }
}
pub fn find_and_process_vmt(game_dir: &Path, base_material: &str) -> anyhow::Result<VmtParams> {
    let options = FileSystemOptions::default();
    let fs = match FileSystem::<DummyVpk>::load_from_path::<P2GameInfo>(game_dir, &options) {
        Some(fs) => fs,
        None => {
            bail!("Failed to load filesystem. Check if gameinfo.txt exists");
        }
    };

    let vmt_data = fs.read(
        format!("materials/{}.vmt", base_material).as_str(),
        "game",
        false
    )
    .map(|v| String::from_utf8_lossy(&v).to_string())
    .context(format!("\"{}\" Not Found", base_material))?;

    let content: crate::vmt_helper::Vmt = source_kv::from_str(&vmt_data)?;
    let data = if content.shader == "patch" {
        match content.properties.get("replace").context("Missing 'replace' block in patch VMT")? {
            crate::vmt_helper::VmtValue::Block(block) => block,
            _ => bail!("Unexpected value for 'replace' block in patch VMT"),
        }
    } else {
        &content.properties
    };

    let parm = VmtParams {
        use_cubemap: data.get("$UseCubemap").and_then(|v| v.as_bool()).unwrap_or_default(),
        reflection_scale: data.get("$ReflectionScale").and_then(|v| v.as_float()).unwrap_or(1.0),
        metalness_scale: data.get("$MetalnessScale").and_then(|v| v.as_float()).unwrap_or(1.0),
        roughness_bias: data.get("$RoughnessBias").and_then(|v| v.as_float()).unwrap_or(1.0),
        uv_scale: data.get("$UV_Scale").and_then(|v| v.as_float()).unwrap_or(1.0),
        ao_scale: data.get("$AO_Scale").and_then(|v| v.as_float()).unwrap_or(1.0),
        global_intensity: data.get("$GlobalIntensity").and_then(|v| v.as_float()).unwrap_or(1.0),
        dielectric_f0: data.get("$DielectricF0").and_then(|v| v.as_float()).unwrap_or(0.04),
        normal_scale: data.get("$NormalScale").and_then(|v| v.as_float()).unwrap_or(1.0),
        fade_start: data.get("$FadeStart").and_then(|v| v.as_float()).unwrap_or(1024.0),
        fade_end: data.get("$FadeEnd").and_then(|v| v.as_float()).unwrap_or(2048.0),
        albedo_tint: [
            data.get("$AlbedoTintR").and_then(|v| v.as_float()).unwrap_or(1.0),
            data.get("$AlbedoTintG").and_then(|v| v.as_float()).unwrap_or(1.0),
            data.get("$AlbedoTintB").and_then(|v| v.as_float()).unwrap_or(1.0),
            data.get("$AlbedoTintRatio").and_then(|v| v.as_float()).unwrap_or(1.0),
        ],
        num_lights: 0.0,
    };

    Ok(parm)
}

/// Generates a Patch VMT that includes the base PBR shader and inserts the generated LUT
pub fn generate_vmt(vmt_path: &Path, texture_rel_path: &str, pbr_material: &str, initial_c4: &[f32; 4], cubemap_path: Option<&str>) -> anyhow::Result<()> {
    if let Some(parent) = vmt_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // TODO: now i have native VMT support, use it!

    let mut file = File::create(vmt_path)?;

    writeln!(file, "patch")?;
    writeln!(file, "{{")?;
    writeln!(file, "\tinclude \"materials/{}.vmt\"", pbr_material)?;
    writeln!(file, "\treplace")?;
    writeln!(file, "\t{{")?;

    // Normalize path separators to forward slashes for Source Engine
    let clean_path = texture_rel_path.replace('\\', "/");
    writeln!(file, "\t\t$texture1 \"{}\"", clean_path)?;

    // Inject Cubemap if available
    if let Some(cpath) = cubemap_path {
        writeln!(file, "\t\t$texture2 \"{}\"", cpath)?;
    }


    // Write $c4 vector based on light initial state
    writeln!(file, "\t\t$c4_x {:.2}", initial_c4[0])?;
    writeln!(file, "\t\t$c4_y {:.2}", initial_c4[1])?;
    writeln!(file, "\t\t$c4_z {:.2}", initial_c4[2])?;
    writeln!(file, "\t\t$c4_w {:.2}", initial_c4[3])?;

    writeln!(file, "\t}}")?;
    writeln!(file, "}}")?;

    Ok(())
}
