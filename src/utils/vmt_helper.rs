use std::collections::HashMap;

use anyhow::Context;
use serde::Deserialize;
use source_fs::{FileSystem, PackFile};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VmtPbrParams {
    #[serde(rename = "$pbrtemplate")]
    pub pbr_shader_template: String,
    #[serde(rename = "$bumpmap")]
    pub bump_map: String,
    #[serde(rename = "$mraotexture")]
    pub mrao_map: String,
    #[serde(rename = "$envmap")]
    pub env_map: Option<String>,

    #[serde(rename = "$usecubemap")]
    pub use_cubemap: bool,

    #[serde(rename = "$reflectionscale")]
    pub reflection_scale: f32,

    #[serde(rename = "$metalnessscale")]
    pub metalness_scale: f32,

    #[serde(rename = "$roughnessbias")]
    pub roughness_bias: f32,

    #[serde(skip)]
    pub num_lights: f32,

    #[serde(rename = "$uv_scale")]
    pub uv_scale: f32,

    #[serde(rename = "$ao_scale")]
    pub ao_scale: f32,

    #[serde(rename = "$globalintensity")]
    pub global_intensity: f32,

    #[serde(rename = "$dielectricf0")]
    pub dielectric_f0: f32,

    #[serde(rename = "$normalscale")]
    pub normal_scale: f32,

    #[serde(rename = "$fadestart")]
    pub fade_start: f32,

    #[serde(rename = "$fadeend")]
    pub fade_end: f32,

    #[serde(rename = "$albetint", deserialize_with = "parse_albedo_tint")]
    pub albedo_tint: [f32; 4],
}

impl Default for VmtPbrParams {
    fn default() -> Self {
        Self {
            pbr_shader_template: String::from("pcapture/shaders/pbs_specular_mrao"),
            bump_map: String::from(""),
            mrao_map: String::from(""),
            env_map: None,
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

impl VmtPbrParams {
    pub fn parse_from_vmt(vmt_data: &str) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct ShaderBody {
            #[serde(rename = "PBR", alias = "pbr")]
            pbr: Option<VmtPbrParams>,
        }

        let parsed: HashMap<String, ShaderBody> = source_kv::from_str(vmt_data)
            .context("Failed to parse VMT structure")?;

        let (_, shader_body) = parsed.into_iter().next()
            .context("VMT is empty or missing a shader block")?;

        Ok(shader_body.pbr.context("Missing PBR block in VMT")?)
    }

    pub fn find_and_parse<P: PackFile>(fs: &FileSystem<P>, base_material: &str) -> anyhow::Result<VmtPbrParams> {
        let vmt_data = fs.read(
            format!("materials/{}.vmt", base_material).as_str(),
            "game",
            true
        )
        .map(|v| String::from_utf8_lossy(&v).to_string())
        .context(format!("\"{}\" Not Found", base_material))?;

        VmtPbrParams::parse_from_vmt(&vmt_data)
    }

    pub fn parse_vmt_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let vmt_data = std::fs::read_to_string(path)?;
        Self::parse_from_vmt(&vmt_data)
    }
}

// Helper for parsing string vectors into an array
fn parse_albedo_tint<'de, D>(deserializer: D) -> Result<[f32; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    let clean_str = s.replace(['[', ']', '{', '}'], "");

    let mut tint = [1.0, 1.0, 1.0, 1.0];
    for (i, val) in clean_str.split_whitespace().filter_map(|x| x.parse::<f32>().ok()).take(4).enumerate() {
        tint[i] = val;
    }

    Ok(tint)
}
