use std::fs::File;
use std::io::{Write, BufWriter};
use std::path::Path;
use byteorder::{WriteBytesExt, LittleEndian};
use anyhow::{Result, Context};

const IMAGE_FORMAT_RGBA32323232F: u32 = 29;
const IMAGE_FORMAT_DXT1: u32 = 13;

/// Flags: POINTSAMPLE | CLAMPS | CLAMPT | NOMIP | NOLOD | HINT_DXT5 | TEXTUREFLAGS_RENDER_TARGET
/// 0x230d = 0010 0011 0000 1101
/// BIT 0: POINT
/// BIT 2: CLAMPS
/// BIT 3: CLAMPT
/// BIT 8: NOMIP
/// BIT 9: NOLOD
/// BIT 13: RENDER_TARGET (0x2000)
const FLAGS: u32 = 0x0000230d;

pub struct VtfParams {
    pub width: u16,
    pub height: u16,
}

/// Writes raw RGBA32F data to a VTF file.
/// `data` must be a flat slice of f32s, 4 per pixel (R, G, B, A).
/// Length of `data` must be width * height * 4.
pub fn write_rgba32f_vtf(path: &Path, params: VtfParams, data: &[f32]) -> Result<()> {
    if data.len() != (params.width as usize * params.height as usize * 4) {
        anyhow::bail!("Data length mismatch. Expected {} floats, got {}",
            params.width as usize * params.height as usize * 4, data.len());
    }

    let f = File::create(path).context("Failed to create VTF file")?;
    let mut writer = BufWriter::new(f);

    // --- Calculate Reflectivity ---
    // Average R, G, B
    let mut sum_r = 0.0;
    let mut sum_g = 0.0;
    let mut sum_b = 0.0;
    let pixel_count = (params.width as u32 * params.height as u32) as f32;

    for chunk in data.chunks_exact(4) {
        sum_r += chunk[0];
        sum_g += chunk[1];
        sum_b += chunk[2];
    }
    let ref_r = sum_r / pixel_count;
    let ref_g = sum_g / pixel_count;
    let ref_b = sum_b / pixel_count;

    // --- Header (96 bytes) ---
    writer.write_all(b"VTF\0")?; // Signature
    writer.write_u32::<LittleEndian>(7)?; // Version[0] (Major)
    writer.write_u32::<LittleEndian>(4)?; // Version[1] (Minor) -> 7.4
    writer.write_u32::<LittleEndian>(96)?; // Header Size
    writer.write_u16::<LittleEndian>(params.width)?;
    writer.write_u16::<LittleEndian>(params.height)?;
    writer.write_u32::<LittleEndian>(FLAGS)?;
    writer.write_u16::<LittleEndian>(1)?; // Frames
    writer.write_u16::<LittleEndian>(0)?; // First Frame
    writer.write_all(&[0u8; 4])?; // Padding

    // Reflectivity (32-44)
    writer.write_f32::<LittleEndian>(ref_r)?;
    writer.write_f32::<LittleEndian>(ref_g)?;
    writer.write_f32::<LittleEndian>(ref_b)?;

    writer.write_all(&[0u8; 4])?; // Padding
    writer.write_f32::<LittleEndian>(1.0)?; // Bump scale
    writer.write_u32::<LittleEndian>(IMAGE_FORMAT_RGBA32323232F)?; // HiRes Format
    writer.write_u8(1)?; // Mip Count
    writer.write_u32::<LittleEndian>(IMAGE_FORMAT_DXT1)?; // LowRes Format
    writer.write_u8(16)?; // LowRes Width
    writer.write_u8(16)?; // LowRes Height
    writer.write_u16::<LittleEndian>(1)?; // Depth

    // Padding (65-67)
    writer.write_all(&[0u8; 3])?;

    // Num Resources (68-71)
    writer.write_u32::<LittleEndian>(2)?;

    // Padding (72-79)
    writer.write_all(&[0u8; 8])?;

    // --- Resource Dictionary (Starts at 80) ---
    // Resource 1: Low Res Image (Thumb)
    // Tag \x01\0\0, Flags 0, Offset 96
    writer.write_all(b"\x01\x00\x00")?;
    writer.write_u8(0)?;
    writer.write_u32::<LittleEndian>(96)?;

    // Resource 2: Image Data
    // Tag \x30\0\0, Flags 0, Offset 224 (96 + 128)
    writer.write_all(b"\x30\x00\x00")?;
    writer.write_u8(0)?;
    writer.write_u32::<LittleEndian>(224)?;

    // --- Body ---

    // 1. Low Res Data (16x16 DXT1 = 128 bytes)
    // Just zeros (black)
    writer.write_all(&[0u8; 128])?;

    // 2. High Res Data (RGBA32323232F)
    for float_val in data {
        writer.write_f32::<LittleEndian>(*float_val)?;
    }

    writer.flush()?;
    Ok(())
}
