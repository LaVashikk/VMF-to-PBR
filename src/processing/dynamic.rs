use std::collections::HashMap;
use vmf_forge::prelude::{Entity, VmfFile};

use crate::types::LightDef;

pub type LightConnectionRegistry = HashMap<u64, Vec<LightConnection>>;

#[derive(Debug)]
pub struct DynamicControllers {
    pub modify_control_entities: Vec<Entity>,
    pub backpatch_connections: HashMap<usize, Vec<(String, String)>>, // entity_id -> vec of connections
}

pub fn build_dynamic_controllers(
    selected_lights: &[(LightDef, f32)],
    cluster_name: &str,
    mat_name: &str,
    origin: &str,
    light_connection_registry: &LightConnectionRegistry,
) -> DynamicControllers {
    let mut modify_control_entities = Vec::new();
    let mut backpatch_connections: HashMap<usize, Vec<(String, String)>> = HashMap::new();

    for (i, (light, _score)) in selected_lights.iter().take(4).enumerate() {
        if !light.is_named_light {
            continue;
        }

        let lookup_key = light.id; // TODO!!!!!!!!! fix it
        let ctrl_name = format!("{}_ctrl_{}", cluster_name, i);

        let mut ctrl_ent = Entity::new("material_modify_control", 100_000);
        ctrl_ent.set("targetname".to_string(), ctrl_name.clone());
        ctrl_ent.set("parentname".to_string(), cluster_name.to_string());
        ctrl_ent.set("materialName".to_string(), mat_name.to_string());

        // Map Index to Variable ($c4_x, y, z, w)
        let var = match i {
            0 => "$c4_x",
            1 => "$c4_y",
            2 => "$c4_z",
            3 => "$c4_w",
            _ => unreachable!()
        };
        ctrl_ent.set("materialVar".to_string(), var.to_string());
        ctrl_ent.set("origin".to_string(), origin.to_string());

        modify_control_entities.push(ctrl_ent);

        if let Some(conns) = light_connection_registry.get(&lookup_key) {
            // Back-patching connections
            log::debug!("Back-patching connections for {}. {:?}", ctrl_name, conns);
            for conn in conns {
                let val = match conn.input_type {
                    LightInputType::TurnOn => "1",
                    LightInputType::TurnOff => "0",
                    // todo: SetPattern
                };
                let new_conn_str = format!("{},SetMaterialVar,{},{},-1", ctrl_name, val, conn.delay);

                backpatch_connections
                    .entry(conn.source_entity_id)
                    .or_default()
                    .push((conn.output_name.clone(), new_conn_str));
            }
        } else {
            log::debug!("lights for {} don't have inputs", ctrl_name);
        }
    }

    DynamicControllers { modify_control_entities, backpatch_connections }
}

#[derive(Debug)]
pub struct LightConnection {
    source_entity_id: usize, // todo: id
    target_name: String,
    output_name: String,
    input_type: LightInputType,
    delay: f32,
}

#[derive(Debug, PartialEq)]
enum LightInputType {
    TurnOn,
    TurnOff,
    // todo: Toggle and SetPattern is complex to handle
}

impl LightConnection {
    fn parse(ent: &Entity) -> Option<Self> {
        let Some(connections) = &ent.connections else { return None; };

        for (output, value) in connections {
            // Parse VMF connection string: "TargetEntity,Input,Param,Delay,Limit"
            let parts: Vec<&str> = value.split([',', '\x1B']).collect(); // TODO: move to vmf-forge!
            let target = parts[0].trim();
            let input = parts[1].trim();
            let delay = parts.get(3).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(0.0);

            let input_type = match input.to_lowercase().as_str() {
                "turnon" => Some(LightInputType::TurnOn),
                "turnoff" => Some(LightInputType::TurnOff),
                _ => None
            };

            if let Some(it) = input_type {
                // if target is "!self" -> then use "ent.targetname()"
                let raw_name = if target.eq_ignore_ascii_case("!self") {
                    let Some(name) = ent.targetname() else { continue };
                    name
                } else {
                    target
                };

                let target_name = raw_name.to_lowercase();

                return Some(LightConnection {
                    source_entity_id: ent.id() as usize,
                    target_name,
                    output_name: output.clone(),
                    input_type: it,
                    delay,
                });
            }
        }

        None
    }
}

pub fn build_connections_registry(vmf: &VmfFile,) -> LightConnectionRegistry {
    let mut light_connection_registry: LightConnectionRegistry = HashMap::new();
    for ent in vmf.entities.iter() {
        if let Some(light_connecting) = LightConnection::parse(ent) {
            let key = ent.id();
            light_connection_registry
                .entry(key)
                .or_default()
                .push(light_connecting);
        }
    }

    light_connection_registry
}
