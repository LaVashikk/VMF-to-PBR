use std::{collections::{HashMap, HashSet}, hash::Hash, sync::Arc};
use vmf_forge::prelude::{Entity, VmfFile};

use crate::types::LightDef;

pub type Connection = Vec<(Arc<str>, String)>;
pub type LightConnectionRegistry = HashMap<u64, Vec<LightConnection>>; // target entity -> connection data

#[derive(Debug)]
pub struct DynamicControllers {
    pub modify_control_entities: Vec<Entity>,
    pub backpatch_connections: HashMap<usize, Connection>, // entity_id -> vec of connections
}

pub fn build_dynamic_controllers(
    selected_lights: &[(LightDef, f32)], // light and score
    cluster_name: &str,
    mat_name: &str,
    origin: &str,
    light_connection_registry: &LightConnectionRegistry,
) -> DynamicControllers {
    let mut modify_control_entities = Vec::new();
    let mut backpatch_connections: HashMap<usize, Connection> = HashMap::new();

    for (i, (light, _score)) in selected_lights.iter().take(4).enumerate() {
        if !light.is_named_light {
            continue;
        }

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

        if let Some(conns) = light_connection_registry.get(&light.id) {
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

#[derive(Debug, Clone)]
pub struct LightConnection {
    // WHO run output
    source_entity_id: usize,
    // what output
    output_name: Arc<str>,
    // what target input
    input_type: LightInputType,
    // optional params
    _parameters: Option<String>,
    // aaand delay! that wasn't obvious fr
    delay: f32,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum LightInputType {
    TurnOn,
    TurnOff,
    // todo: Toggle and SetPattern is complex to handle
}

pub fn build_connections_registry(vmf: &VmfFile,) -> LightConnectionRegistry {
    let mut targetname_to_ids: HashMap<String, Vec<u64>> = HashMap::new();
    for ent in vmf.entities.iter() {
        if let Some(name) = ent.targetname() {
            targetname_to_ids.entry(name.to_lowercase()).or_default().push(ent.id());
        }
    }

    let mut light_connection_registry: LightConnectionRegistry = HashMap::new();
    let mut outputs_cache: HashSet<Arc<str>> = HashSet::new();

    for ent in vmf.entities.iter() {
        let Some(connections) = ent.connections.as_ref() else { continue };

        for (output, value) in connections.iter() {
            // Parse VMF connection string: "TargetEntity,Input,Param,Delay,Limit"
            let parts: Vec<&str> = value.split([',', '\x1B']).collect(); // TODO: move to vmf-forge!
            let target = parts[0].trim();
            let input = parts[1].trim();
            let parameters = Some(parts[2].trim()).filter(|s| !s.is_empty()).map(|s| s.to_string());
            let delay = parts.get(3).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(0.0);

            // Use Arc for caching output names
            let output_arc = match outputs_cache.get(output.as_str()) {
                Some(existing) => existing.clone(),
                None => {
                    let new_arc: Arc<str> = Arc::from(output.as_str());
                    outputs_cache.insert(new_arc.clone());
                    new_arc
                }
            };

            let input_type = match input.to_lowercase().as_str() {
                "turnon" => LightInputType::TurnOn,
                "turnoff" => LightInputType::TurnOff,
                _ => continue
            };

            // if target is "!self" -> then use "ent.targetname()"
            let target_name = if target.eq_ignore_ascii_case("!self") {
                let Some(name) = ent.targetname() else { continue };
                name
            } else {
                target
            }.to_lowercase();

            let Some(target_ids) = targetname_to_ids.get(&target_name) else { continue };

            for id in target_ids {
                let key = *id;
                let conn = LightConnection {
                    source_entity_id: ent.id() as usize,
                    output_name: output_arc.clone(),
                    input_type,
                    _parameters: parameters.clone(),
                    delay,
                };

                light_connection_registry
                    .entry(key)
                    .or_default()
                    .push(conn);
            }
        }
    }


    light_connection_registry
}

pub fn apply_dynamic_controllers(vmf: &mut VmfFile, clusters: &[crate::LightCluster], light_connection_registry: &LightConnectionRegistry) {
    for cluster in clusters {
        let controllers = build_dynamic_controllers(
            &cluster.lights,
            &cluster.ggx_surface_name,
            &cluster.pbr_material,
            &cluster.bound.center.to_origin(),
            &light_connection_registry
        );

        // Add 'modify_control_entities'. These entities are responsible for changing
        // values in c4 register slots, controlling light brightness.
        vmf.entities.extend(controllers.modify_control_entities);

        // Backpatch connections to existing entities. This integrates the new control
        // mechanisms by adding output connections to control created 'modify_control'.
        for (src_id, conns) in controllers.backpatch_connections {
            vmf.entities.find_by_keyvalue_mut("id", &src_id.to_string()).for_each(|ent| {
                let Some(connections) = &mut ent.connections else { return };
                for connect in conns.iter() {
                    connections.push((connect.0.to_string(), connect.1.clone()));
                }
            });
        }
    }
}
