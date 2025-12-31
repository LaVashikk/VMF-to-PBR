use vmf_forge::{prelude::Entity, VmfFile};

pub struct LightSettings {
    id: u64,
    target_name: String,
}

impl LightSettings {
   pub fn new(ent: &Entity) -> Self {
       Self {
           id: ent.id(),
           target_name: ent.targetname().map(|s| s.to_string()).unwrap_or_default()
       }
   }

   pub fn save(vmf: &mut VmfFile) {
       todo!()
   }
}


pub struct DebugControlsState {
    pub enabled: bool,
    pub draw_ggx_surfaces: bool,
    pub draw_light_surface_links: bool,
    pub draw_rejected_light_sources: bool,
    pub draw_blockers: bool,
    pub inspect_surface: bool,
}
impl Default for DebugControlsState {
    fn default() -> Self {
        Self {
            enabled: true,
            draw_ggx_surfaces: true,
            draw_light_surface_links: true,
            draw_rejected_light_sources: false,
            draw_blockers: false,
            inspect_surface: false,
        }
    }
}

macro_rules! iterable_struct {
    // Паттерн перехватывает:
    // 1. Атрибуты (например, #[derive(Debug)])
    // 2. Видимость (pub)
    // 3. Имя структуры
    // 4. Поля и их типы
    (
        $(#[$meta:meta])* $vis:vis struct $name:ident {
            $($field_vis:vis $field:ident : $type:ty),* $(,)?
        }
    ) => {
        // 1. Генерируем саму структуру как обычно
        $(#[$meta])*
        $vis struct $name {
            $($field_vis $field : $type),*
        }

        // 2. Автоматически реализуем метод iter_mut
        impl $name {
            #[allow(dead_code)]
            pub fn iter_mut(&mut self) -> Vec<(&'static str, &mut dyn std::any::Any)> {
                vec![
                    // Генерируем кортеж (имя, ссылка) для каждого поля
                    $(
                        (stringify!($field), &mut self.$field as &mut dyn std::any::Any)
                    ),*
                ]
            }
        }
    };
}

iterable_struct! {
/// Raw key-value pairs directly from the VMF entity.
#[derive(Debug, Clone)]
pub struct VmfRawLightData {
    pub has_changed: bool,
    // --- System ---
    pub vmf_id: String,
    pub classname: String,
    pub targetname: Option<String>,
    pub spawnflags: String,
    pub origin: String,
    pub angles: String,
    pub light: Option<String>,

    // --- Attenuation (Falloff) ---
    pub fifty_percent_distance: Option<String>,
    pub zero_percent_distance: Option<String>,
    pub constant_attn: Option<String>, // Legacy
    pub linear_attn: Option<String>,   // Legacy
    pub quadratic_attn: Option<String>, // Legacy

    // --- Spotlight Shapes ---
    pub pitch: Option<String>,
    pub inner_cone: Option<String>,
    pub cone: Option<String>,
    pub exponent: Option<String>,

    // --- PBR Custom Keys ---
    pub pbr_enabled: Option<String>,
    pub pbr_intensity_scale: Option<String>,
    pub pbr_color_override: Option<String>,
    pub pbr_range_override: Option<String>,
    pub pbr_blocker_name: Option<String>,
    pub pbr_blocker_name_2: Option<String>,
    pub pbr_bidirectional: Option<String>,
}}

impl VmfRawLightData {
    pub fn new(ent: &Entity) -> Self {
        VmfRawLightData {
            has_changed: false,
            vmf_id: ent.id().to_string(),
            classname: ent.classname().unwrap_or("unknown").to_string(),
            targetname: ent.targetname().map(|s| pbr_lut_gen::parser::sanitize_name(s)),
            origin: ent.get("origin").cloned().unwrap_or_default(),
            angles: ent.get("angles").cloned().unwrap_or_default(),
            pitch: ent.get("pitch").cloned(),
            spawnflags: ent.get("spawnflags").cloned().unwrap_or_else(|| "0".to_string()),

            // Macros/Helper would be nice here, but explicit is clear:
            light: ent.get("_light").cloned(),

            fifty_percent_distance: ent.get("_fifty_percent_distance").cloned(),
            zero_percent_distance: ent.get("_zero_percent_distance").cloned(),
            constant_attn: ent.get("_constant_attn").cloned(),
            linear_attn: ent.get("_linear_attn").cloned(),
            quadratic_attn: ent.get("_quadratic_attn").cloned(),

            inner_cone: ent.get("_inner_cone").cloned(),
            cone: ent.get("_cone").cloned(),
            exponent: ent.get("_exponent").cloned(),

            pbr_enabled: ent.get("pbr_enabled").cloned(),
            pbr_intensity_scale: ent.get("pbr_intensity_scale").cloned(),
            pbr_color_override: ent.get("pbr_color_override").cloned(),
            pbr_range_override: ent.get("pbr_range_override").cloned(),
            pbr_blocker_name: ent.get("pbr_blocker_name").cloned(),
            pbr_blocker_name_2: ent.get("pbr_blocker_name_2").cloned(),
            pbr_bidirectional: ent.get("pbr_bidirectional").cloned(),
        }
    }

    pub fn apply_to_entity(&self, ent: &mut Entity) {
        ent.set("classname".to_string(), self.classname.clone());
        ent.set("origin".to_string(), self.origin.clone());
        ent.set("angles".to_string(), self.angles.clone());
        ent.set("spawnflags".to_string(), self.spawnflags.clone());

        if let Some(val) = &self.targetname {
            ent.set("targetname".to_string(), val.clone());
        } else {
            ent.remove_key("targetname");
        }

        if let Some(val) = &self.pitch {
            ent.set("pitch".to_string(), val.clone());
        }

        let mut set_opt = |key: &str, val: &Option<String>| {
            if let Some(v) = val {
                ent.set(key.to_string(), v.clone());
            }
        };

        // --- Standard Lighting ---
        set_opt("_light", &self.light);

        // --- Attenuation ---
        set_opt("_fifty_percent_distance", &self.fifty_percent_distance);
        set_opt("_zero_percent_distance", &self.zero_percent_distance);
        set_opt("_constant_attn", &self.constant_attn);
        set_opt("_linear_attn", &self.linear_attn);
        set_opt("_quadratic_attn", &self.quadratic_attn);

        // --- Spot Shape ---
        set_opt("_inner_cone", &self.inner_cone);
        set_opt("_cone", &self.cone);
        set_opt("_exponent", &self.exponent);

        // --- PBR Custom Keys ---
        set_opt("pbr_enabled", &self.pbr_enabled);
        set_opt("pbr_intensity_scale", &self.pbr_intensity_scale);
        set_opt("pbr_color_override", &self.pbr_color_override);
        set_opt("pbr_range_override", &self.pbr_range_override);
        set_opt("pbr_blocker_name", &self.pbr_blocker_name);
        set_opt("pbr_blocker_name_2", &self.pbr_blocker_name_2);

        set_opt("pbr_bidirectional", &self.pbr_bidirectional);
    }

    pub fn to_entity(&self) -> Entity {
        let mut ent = Entity::new(&self.classname, 0);
        self.apply_to_entity(&mut ent);
        ent
    }
}
