// Defines the material that identifies faces to be patched
pub const TARGET_MATERIAL: &str = "tools/toolspbr";

// Apply to move PBR solids closer to the albedo surface (in hammer units)
pub const GEOMETRY_OFFSET_UNITS: f32 = 0.8;

// Search distance for find albedo surface to fix UV (in hammer units)
pub const UV_SEARCH_DIST: f32 = 16.0;

// Width of the LUT (in pixels)
pub const LUT_WIDTH: usize = 8;

// Height of the LUT (in pixels)
pub const LUT_HEIGHT: usize = 16;
