#![allow(dead_code)]
#![allow(unused_variables)]

use std::{path::{Path, PathBuf}, sync::mpsc};

use egui::{Color32, RichText, Slider, TextEdit};
use source_sdk::Engine;
use vmf_forge::VmfFile;
use crate::{SharedState, Window};

mod types;
use types::*;

pub struct PbrDebug {
    // Window state
    is_open: bool,
    vmf: Option<VmfFile>,
    lights_data: Option<Vec<VmfRawLightData>>,

    debug_state: DebugControlsState,
    selected_light_idx: usize,
    last_value: String,
    next_update: f32,

    picked_path: Option<PathBuf>,
    continue_anyway: bool,
    file_dialog_receiver: mpsc::Receiver<Option<PathBuf>>,
    file_dialog_sender: mpsc::Sender<Option<PathBuf>>,
}

impl PbrDebug {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            is_open: true,
            vmf: None,

            debug_state: DebugControlsState::default(),
            lights_data: None,
            selected_light_idx: 0,
            last_value: String::default(),
            next_update: 0.0,

            picked_path: Some(PathBuf::from("/home/lavashik/Documents/PCapture/MapsSRC/Act1/ready/PCap_A1_03.vmf")), // todo debug
            // picked_path: None,
            continue_anyway: false,
            file_dialog_receiver: rx,
            file_dialog_sender: tx,
        }
    }

    fn open_vmf(&mut self, path: &PathBuf, engine: &Engine) {
        let vmf = VmfFile::open(path).unwrap();
        let mut lights = Vec::with_capacity(256);
        engine.client().execute_client_cmd_unrestricted("script_execute PCapture-Core/pbr_debug");
        for ent in vmf.entities.iter() {
            let classname = ent.classname().unwrap_or("");
            if classname != "light" && classname != "light_spot" && classname != "func_ggx_area" { continue }
            lights.push(VmfRawLightData::new(ent));
        }

        self.vmf = Some(vmf);
        self.lights_data = Some(lights);
    }

    fn save_vmf(&mut self) {
        todo!()
    }

    /// Drawing select for vmf map // TODO!
    fn draw_vmf_select(&mut self, ctx: &egui::Context, engine: &Engine) {
        let modal_id = egui::Id::new("vmf_picker_modal");
        let area = egui::Modal::default_area(modal_id)
            .default_size(ctx.screen_rect().size() * 0.5);
        let modal = egui::Modal::new(modal_id)
            .frame(egui::Frame::NONE)
            .area(area);

        modal.show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());

                ui.heading("Select a VMF Map File");
                if let Some(path) = self.picked_path.clone() {
                    let current_level = engine.client().get_level_name_short();
                    let has_match = path.file_name()
                            .and_then(|name| name.to_str())
                            .map(|name_str| name_str.contains(&current_level))
                            .unwrap_or(false);

                    if has_match || self.continue_anyway {
                        self.open_vmf(&path, engine);
                        self.vmf = Some(VmfFile::open(path).unwrap());
                        self.continue_anyway = false;
                    } else {
                        ui.label(RichText::new("Warning: The selected file does not match the loaded map name")
                            .color(egui::Color32::RED)
                            // .small()
                        );

                        if ui.button("Proceed anyway").clicked() {
                            self.continue_anyway = true;
                        }
                    }
                } else {
                    ui.label("No file selected.");
                }

                ui.add_space(20.0);

                if ui.button("Open File Dialog...").clicked() {
                    // Spawn a new thread for the file dialog
                    let sender = self.file_dialog_sender.clone();
                    std::thread::spawn(move || {
                        let file = rfd::FileDialog::new()
                            .add_filter("Valve Map File", &["vmf"])
                            .set_directory(".")
                            .pick_file();

                        // Send the result back to the main thread.
                        let _ = sender.send(file);
                    });
                }

            });
        });
    }

    /// Drawing main debug panel
    fn draw_debug(&mut self, ctx: &egui::Context, engine: &Engine) {
        egui::Area::new("debug_controls_panel".into())
            .default_height(ctx.screen_rect().height() * 0.5)
            .anchor(egui::Align2::LEFT_CENTER, egui::vec2(10.0, 0.0))
            .show(ctx, |ui| {
                let panel_frame = egui::Frame {
                    inner_margin: egui::Margin::same(10),
                    fill: egui::Color32::from_rgba_premultiplied(30, 30, 30, 240),
                    stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    ..egui::Frame::window(ui.style())
                };

                // Now, show the frame and add the content inside it.
                panel_frame.show(ui, |ui| {
                    // ui.set_height(ui.available_height());
                    ui.spacing_mut().item_spacing.y = 12.0;
                    let layout = egui::Layout::top_down(egui::Align::Min).with_main_justify(true);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);

                    ui.heading("Debug settings");
                    ui.add_space(10.0);

                    if ui.toggle_value(&mut self.debug_state.enabled, "Enable").changed() {
                        if self.debug_state.enabled {
                            engine.client().client_cmd("script START()");
                        } else {
                            engine.client().client_cmd("script STOP()");
                        }
                    }
                    // todo: simpl this
                    if ui.checkbox(&mut self.debug_state.draw_ggx_surfaces, "Show GGX Surfaces").changed() {
                        engine.client().client_cmd(&format!("script DRAW_SURFACE = {}", self.debug_state.draw_ggx_surfaces));
                    }
                    if ui.checkbox(&mut self.debug_state.draw_light_surface_links, "Visualize Light Links").changed() {
                        engine.client().client_cmd(&format!("script DRAW_LINK = {}", self.debug_state.draw_light_surface_links));
                    }
                    if ui.checkbox(&mut self.debug_state.draw_rejected_light_sources, "Visualize Culled Lights",).changed() {
                        engine.client().client_cmd(&format!("script DRAW_REJECTED = {}", self.debug_state.draw_rejected_light_sources));
                    }
                    if ui.checkbox(&mut self.debug_state.draw_blockers, "Show Occluders").changed() {
                        engine.client().client_cmd(&format!("script DRAW_BLOCKER = {}", self.debug_state.draw_blockers));
                    }
                    if ui.checkbox(&mut self.debug_state.inspect_surface, "Inspect Mode").changed() {
                        engine.client().client_cmd(&format!("script INSPECT_SURFACE = {}", self.debug_state.inspect_surface));
                    }

                    ui.add_space(10.0);

                    let time = engine.client().get_last_time_stamp();
                    if time >= self.next_update { //ui.button("Update").clicked() {
                        // TODO temp update
                        let vmf = self.vmf.as_mut().unwrap();
                        let lights = self.lights_data.as_mut().unwrap();
                        let mut is_need_update = false;
                        self.next_update = time + 0.25;

                        for light in lights {
                            if !light.has_changed { continue }
                            is_need_update = true;
                            light.has_changed = false;

                            log::warn!("[{}] PROCESSING {} ({:?})", time, light.vmf_id, light.targetname);
                            let mut founded = false;
                            for original_ent in vmf.entities.find_by_keyvalue_mut("id", &light.vmf_id) {
                                light.apply_to_entity(original_ent);
                                founded = true; // todo: temp

                            }
                            if !founded {
                                log::warn!("NOT FOUND ENTITY IN VMF: {}", light.vmf_id);
                            }
                        }

                        if is_need_update {
                            let all_lights = pbr_lut_gen::parser::extract_lights(&vmf).unwrap(); // todo
                            let game_dir = Path::new("/home/lavashik/.local/share/Steam/steamapps/common/Portal 2/portal2/"); // TODO
                            let map_name = self.picked_path.as_ref().unwrap().file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap() // todo
                                    .to_string();
                            let clusters = pbr_lut_gen::processing::process_map_pipeline(
                                &mut vmf.clone(), // todo
                                &all_lights,
                                &game_dir,
                                &map_name,
                                false
                            ).unwrap();

                            let nut_path = game_dir
                                .join("scripts/vscripts/_autogen_debug")
                                .join(format!("{}_pbr.nut", map_name));
                            pbr_lut_gen::nut_gen::generate_nut(&nut_path, &clusters, &all_lights).unwrap();

                            engine.client().client_cmd("script UPD()");
                        }
                    }

                    ui.separator();
                    ui.add_space(4.0);

                    if ui.button("Load VMF").clicked() {
                        // Your 'Load VMF' logic here...
                    }
                    if ui.button("Save VMF").clicked() {
                        // Your 'Save VMF' logic here...
                    }
                });
            });


        let panel_frame = egui::Frame {
            fill: egui::Color32::from_rgba_premultiplied(30, 30, 30, 240),
            ..egui::Frame::side_top_panel(ctx.style().as_ref())
        };

        egui::SidePanel::right("properties_panel")
            .frame(panel_frame)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();
                ui.add_space(10.0);

                // Add the requested content in one line
                let light = [0];
                if let Some(light) = self.lights_data.as_mut().unwrap().get_mut(self.selected_light_idx) {
                    property_editor_ui(ui, light);
                }

                let selected_light = if let Some(val) = engine.cvar_system().find_var("#pbr_current_selected") {
                    val.get_string()
                } else {
                    log::error!("THERE'S NO CVAR!!");
                    return;
                };

                if selected_light != self.last_value {
                    log::info!("selected: {}", &selected_light);
                    if let Some(idx) = self.lights_data.as_deref()
                        .and_then(|lights| lights.iter().position(|val| val.targetname.as_deref() == Some(&selected_light) || val.vmf_id == selected_light))
                    {
                        self.selected_light_idx = idx;
                    }
                    self.last_value = selected_light;
                }
            });

    }

    fn process_debug_logic(&mut self, ctx: &egui::Context, engine: &Engine) {

    }

}


impl Window for PbrDebug {
    fn name(&self) -> &'static str { "PBR Debug" }
    fn toggle(&mut self) { self.is_open = !self.is_open; }
    fn is_open(&self) -> bool { self.is_open }
    fn is_should_render(&self, shared_state: &SharedState, _engine: &source_sdk::Engine) -> bool {
        shared_state.is_overlay_focused
    }
    fn draw(&mut self, ctx: &egui::Context, _shared_state: &mut SharedState, engine: &Engine) {
        if let Ok(picked_file) = self.file_dialog_receiver.try_recv() {
            self.picked_path = picked_file;
        }

        if let Some(vmf) = &self.vmf {
            self.draw_debug(ctx, engine);
        } else {
            self.draw_vmf_select(ctx, engine);
        }
    }
}


pub fn property_editor_ui(ui: &mut egui::Ui, light: &mut VmfRawLightData) {
    let is_area = light.classname == "func_ggx_area";
    let is_spot = light.classname == "light_spot";
    let mut any_changed = false;

    egui::Grid::new("property_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for (name, val) in light.iter_mut() {
                if !is_area && name == "pbr_bidirectional" { continue }
                if !is_spot && matches!(name, "pitch" | "inner_cone" | "cone" | "exponent") { continue }

                ui.label(name);
                match name {
                    "light" | "pbr_color_override" => {
                        if let Some(opt_s) = val.downcast_mut::<Option<String>>() {
                            let s = opt_s.clone().unwrap_or_default();
                            let (mut color, mut brightness) = parse_color_brightness(&s);

                            let response = ui.horizontal(|ui| {
                                let mut rgb = [color.r(), color.g(), color.b()];
                                let color_changed = ui.color_edit_button_srgb(&mut rgb).changed();
                                if color_changed {
                                    color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                                }

                                let brightness_changed = ui.add(Slider::new(&mut brightness, 0.0..=10000.0)).changed();
                                color_changed || brightness_changed
                            });

                            if response.inner {
                                any_changed = true;
                                let new_s = format_color_brightness(color, brightness);
                                *opt_s = Some(new_s);
                            }
                        }
                    }

                    "classname" => {
                        if let Some(classname_str) = val.downcast_mut::<String>() {
                            let before = classname_str.clone();
                            egui::ComboBox::from_id_salt(name)
                                .selected_text(classname_str.as_str())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(classname_str, "light".to_string(), "light");
                                    ui.selectable_value(classname_str, "light_spot".to_string(), "light_spot");
                                    ui.selectable_value(classname_str, "func_ggx_area".to_string(), "func_ggx_area");
                                });
                            if *classname_str != before {
                                any_changed = true;
                            }
                        }
                    }

                    "pbr_enabled" => {
                        if let Some(opt_s) = val.downcast_mut::<Option<String>>() {
                            let mut is_enabled = opt_s.as_deref() == Some("1");
                            if ui.checkbox(&mut is_enabled, "").changed() {
                                any_changed = true;
                                *opt_s = Some((if is_enabled { "1" } else { "0" }).to_string());
                            }
                        }
                    }

                    "fifty_percent_distance" => {
                        if let Some(opt_s) = val.downcast_mut::<Option<String>>() {
                            let s = opt_s.clone().unwrap_or_default();
                            let mut value: f32 = s.parse().unwrap_or(256.0);

                            if ui.add(Slider::new(&mut value, 0.0..=4096.0).logarithmic(true)).changed() {
                                any_changed = true;
                                *opt_s = Some(value.to_string());
                            }
                        }
                    }

                    "pbr_bidirectional" => {
                        if let Some(opt_s) = val.downcast_mut::<Option<String>>() {
                            let mut current_value = opt_s.as_deref().unwrap_or("0").to_string();
                            let before = current_value.clone();
                            let selected_text = if current_value == "1" { "Yes (Two Sided)" } else { "No (One Sided)" };

                            egui::ComboBox::from_id_salt(name)
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut current_value, "0".to_string(), "No (One Sided)");
                                    ui.selectable_value(&mut current_value, "1".to_string(), "Yes (Two Sided)");
                                });

                            if current_value != before {
                                any_changed = true;
                            }
                            *opt_s = Some(current_value);
                        }
                    }

                    // Обработка всех остальных полей как простого текста
                    _ => {
                        if let Some(s) = val.downcast_mut::<String>() {
                            if ui.add(TextEdit::singleline(s)).changed() {
                                any_changed = true;
                            }
                        }
                        else if let Some(opt_s) = val.downcast_mut::<Option<String>>() {
                            let mut text = opt_s.clone().unwrap_or_default();
                            if ui.add(TextEdit::singleline(&mut text)).changed() {
                                any_changed = true;
                                *opt_s = if text.is_empty() { None } else { Some(text) };
                            }
                        }
                    }
                }

                ui.end_row();
            }
        });

    if !light.has_changed {
        light.has_changed = any_changed;
    }
}

// Функция для парсинга цвета и яркости из строки "R G B Brightness"
fn parse_color_brightness(s: &str) -> (Color32, f32) {
    let parts: Vec<f32> = s
        .split_whitespace()
        .filter_map(|num| num.parse().ok())
        .collect();
    if parts.len() == 4 {
        (
            Color32::from_rgb(parts[0] as u8, parts[1] as u8, parts[2] as u8),
            parts[3],
        )
    } else {
        (Color32::WHITE, 200.0) // Значения по умолчанию
    }
}

// Функция для форматирования обратно в строку
fn format_color_brightness(color: Color32, brightness: f32) -> String {
    format!("{} {} {} {}", color.r(), color.g(), color.b(), brightness)
}
