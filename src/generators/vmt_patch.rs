use std::{fs::File, path::Path};
use std::io::Write;
use crate::vmt_helper::VmtPbrParams;

/// Generates a Patch VMT that includes the base PBR shader and inserts the generated LUT
pub fn generate(vmt_path: &Path, texture_rel_path: &str, params: &VmtPbrParams, initial_c4: &[f32; 4], cubemap_path: Option<&str>) -> anyhow::Result<()> {
    if let Some(parent) = vmt_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // TODO: now i have native VMT support, use it!

    let mut file = File::create(vmt_path)?;

    writeln!(file, "patch")?;
    writeln!(file, "{{")?;
    writeln!(file, "\tinclude \"materials/{}.vmt\"", params.pbr_shader_template)?; // preprocess materials/vmt path
    writeln!(file, "\treplace")?;
    writeln!(file, "\t{{")?;

    // Normalize path separators to forward slashes for Source Engine
    let clean_path = texture_rel_path.replace('\\', "/");
    writeln!(file, "\t\t$basetexture \"{}\"", params.bump_map)?;
    writeln!(file, "\t\t$texture1 \"{}\"", clean_path)?;

    // Inject Cubemap if available
    if let Some(env_map) = params.env_map.as_ref() {
        writeln!(file, "\t\t$texture2 \"{}\"", env_map)?;
    } else if let Some(cpath) = cubemap_path {
        writeln!(file, "\t\t$texture2 \"{}\"", cpath)?;
    }

    writeln!(file, "\t\t$texture3 \"{}\"", params.mrao_map)?;


    // Write $c4 vector based on light initial state
    writeln!(file, "\t\t$c4_x {:.2}", initial_c4[0])?;
    writeln!(file, "\t\t$c4_y {:.2}", initial_c4[1])?;
    writeln!(file, "\t\t$c4_z {:.2}", initial_c4[2])?;
    writeln!(file, "\t\t$c4_w {:.2}", initial_c4[3])?;

    writeln!(file, "\t}}")?;
    writeln!(file, "}}")?;

    Ok(())
}
