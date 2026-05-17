use std::{fs::File, path::Path};
use std::io::Write;
use serde::{Deserialize, Serialize};
use source_vmt::Vmt;

use crate::vmt_helper::VmtPbrParams;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct ReplaceBlock {
    #[serde(rename = "$basetexture")]
    normal_map: String,
    #[serde(rename = "$texture1")]
    lut_map: String,
    #[serde(rename = "$texture2")]
    cubemap: Option<String>,
    #[serde(rename = "$texture3")]
    mrao: String,

    #[serde(rename = "$c4_x")]
    light_style_x: f32,
    #[serde(rename = "$c4_y")]
    light_style_y: f32,
    #[serde(rename = "$c4_z")]
    light_style_z: f32,
    #[serde(rename = "$c4_w")]
    light_style_w: f32,
}

// Generates a Patch VMT that includes the base PBR shader and inserts the generated LUT
pub fn generate(vmt_path: &Path, texture_rel_path: &str, params: &VmtPbrParams, initial_c4: &[f32; 4], cubemap_path: Option<&str>) -> anyhow::Result<()> {
    if let Some(parent) = vmt_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let clean_path = texture_rel_path.replace('\\', "/");

    // Inject Cubemap if available
    let cubemap = if let Some(env_map) = params.env_map.as_ref() {
        Some(env_map.clone())
    } else if let Some(cpath) = cubemap_path {
        Some(cpath.to_string())
    } else { None };

    let replace_block = ReplaceBlock {
        normal_map: params.bump_map.clone(),
        lut_map: clean_path,
        cubemap,
        mrao: params.mrao_map.clone(),
        light_style_x: initial_c4[0],
        light_style_y: initial_c4[1],
        light_style_z: initial_c4[2],
        light_style_w: initial_c4[3],
    };

    let mut vmt = Vmt::new("patch");
    vmt.set_string("include", &format!("materials/{}.vmt", params.pbr_shader_template));
    vmt.set_block("replace", &replace_block)?;

    vmt.to_file(&vmt_path)?;

    Ok(())
}
