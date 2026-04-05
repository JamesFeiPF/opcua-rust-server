use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::io::Cursor;

use opcua::server::address_space::Variable;
use opcua::server::node_manager::memory::{simple_node_manager, InMemoryNodeManager, SimpleNodeManagerImpl};
use opcua::server::ServerBuilder;
use opcua::types::{BuildInfo, DataValue, DateTime, NodeId};
use opcua_nodes::AccessLevel;

const UPDATE_INTERVAL_MS: u64 = 100;

static ACTIVE_SUBSCRIPTIONS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_SESSIONS: AtomicUsize = AtomicUsize::new(0);

fn make_access_level(readable: bool, writable: bool) -> AccessLevel {
    let mut bits = AccessLevel::empty();
    if readable { bits.insert(AccessLevel::CURRENT_READ); }
    if writable { bits.insert(AccessLevel::CURRENT_WRITE); }
    bits
}

use rand::{Rng, SeedableRng};
use rand::rngs::SmallRng;
use parking_lot::RwLock;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct TagConfig {
    name: String,
    node_id: String,
    data_type: String,
    initial_value: f64,
    description: String,
    unit: String,
    readonly: bool,
    changerate: f64,
}

#[derive(Clone)]
struct TagInfo {
    node_id: NodeId,
    current_value: f64,
    display_name: String,
    description: String,
    unit: String,
    readonly: bool,
    changerate: f64,
}

fn get_exe_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn read_csv_tags() -> Vec<TagInfo> {
    let exe_dir = get_exe_dir();
    let csv_path = exe_dir.join("tags.csv");
    let mut tags = Vec::new();

    // Try to read new format first
    if let Ok(mut reader) = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&csv_path)
    {
        for result in reader.deserialize() {
            if let Ok(config) = result {
                let config: TagConfig = config;
                // Parse node_id string like "ns=2;s=TagName" to NodeId
                let node_id = if config.node_id.starts_with("ns=") {
                    let parts: Vec<&str> = config.node_id.split(";s=").collect();
                    if parts.len() == 2 {
                        if let Ok(ns) = parts[0][3..].parse::<u16>() {
                            NodeId::new(ns, parts[1].to_string())
                        } else {
                            NodeId::new(2, config.node_id.clone())
                        }
                    } else {
                        NodeId::new(2, config.node_id.clone())
                    }
                } else {
                    NodeId::new(2, config.node_id.clone())
                };
                tags.push(TagInfo {
                    node_id,
                    current_value: config.initial_value,
                    display_name: config.name,
                    description: config.description,
                    unit: config.unit,
                    readonly: config.readonly,
                    changerate: config.changerate,
                });
                continue;
            }
        }
    }

    // If new format failed or empty, fall back to old format
    if tags.is_empty() {
        println!("Using legacy CSV format");
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(&csv_path)
            .expect(&format!("Failed to open CSV: {:?}", csv_path));

        for (i, result) in reader.records().enumerate() {
            let record = result.expect("Failed to read CSV record");
            let initial_value = record.get(4).unwrap_or("0").parse().unwrap_or(0.0);
            tags.push(TagInfo {
                node_id: NodeId::new(2, format!("Tag_{}", i)),
                current_value: initial_value,
                display_name: record.get(1).unwrap_or("").to_string(),
                description: record.get(2).unwrap_or("").to_string(),
                unit: record.get(3).unwrap_or("").to_string(),
                readonly: true, // Legacy tags are readonly by default
                changerate: 0.3, // Default change rate
            });
        }
    }

    tags
}

#[tokio::main]
async fn main() {
    let exe_dir = get_exe_dir();
    let config_path = exe_dir.join("server.conf");

    println!("Starting Rust OPC UA ReadWrite Server...");

    let (server, handle) = ServerBuilder::new()
        .with_config_from(config_path.to_str().unwrap())
        .max_sessions(100)
        .build_info(BuildInfo {
            product_uri: "urn:RustOPCUA:ReadWriteServer".into(),
            manufacturer_name: "Rust OPC UA".into(),
            product_name: "Rust OPC UA ReadWrite Server".into(),
            software_version: "0.2.0".into(),
            build_number: "2".into(),
            build_date: DateTime::now(),
        })
        .with_node_manager(simple_node_manager(
            opcua::server::diagnostics::NamespaceMetadata {
                namespace_uri: "urn:ReadWriteServer".to_owned(),
                ..Default::default()
            },
            "readwrite",
        ))
        .trust_client_certs(true)
        .build()
        .unwrap();

    let node_manager = handle
        .node_managers()
        .get_of_type::<InMemoryNodeManager<SimpleNodeManagerImpl>>()
        .unwrap();

    println!("Loading tags from CSV...");
    let tags = read_csv_tags();
    let total_tags = tags.len();
    println!("Loaded {} tags", total_tags);

    let ns = handle.get_namespace_index("urn:ReadWriteServer").unwrap();

    let tags_shared: Arc<RwLock<Vec<TagInfo>>> = Arc::new(RwLock::new(tags.clone()));
    let tags_for_api = tags_shared.clone();

    {
        let address_space = node_manager.address_space();
        let mut address_space = address_space.write();
        let folder_id = NodeId::new(ns, "ReadWriteSimulationFolder");
        address_space.add_folder(&folder_id, "ReadWrite Simulation", "ReadWrite Simulation", &NodeId::objects_folder_id());

        let vars: Vec<Variable> = tags.iter().map(|tag| {
            let mut variable = Variable::new(&tag.node_id, &tag.display_name, &tag.description, tag.current_value);

            // Set access level based on readonly flag
            let access_level = make_access_level(true, !tag.readonly);
            variable.set_access_level(access_level);
            variable.set_user_access_level(access_level);

            variable
        }).collect();
        let _ = address_space.add_variables(vars, &folder_id);
    }

    println!("Created {} nodes", total_tags);

    let tags_clone = tags.clone();
    let manager = node_manager.clone();
    let subscriptions = handle.subscriptions().clone();
    let running = Arc::new(AtomicBool::new(true));
    let running_for_update = running.clone();
    let running_for_ctrlc = running.clone();

    let update_counter = Arc::new(AtomicUsize::new(0));
    let update_counter_clone = update_counter.clone();
    let tags_for_update = tags_shared.clone();
    let manager_for_update = manager.clone();
    let subs_for_update = subscriptions.clone();

    // Background task for simulating data changes
    tokio::spawn(async move {
        let mut rng = SmallRng::from_entropy();
        let mut interval = tokio::time::interval(Duration::from_millis(UPDATE_INTERVAL_MS));

        loop {
            interval.tick().await;
            let tags = tags_for_update.read();
            let total = tags.len();

            let mut updates: Vec<(&NodeId, Option<&opcua::types::NumericRange>, DataValue)> = Vec::new();

            for tag in tags.iter() {
                if tag.changerate > 0.0 {
                    // Calculate change based on change rate
                    let change_pct = (rng.gen::<f64>() - 0.5) * tag.changerate * 0.01;
                    let new_value = tag.current_value * (1.0 + change_pct);

                    updates.push((&tag.node_id, None, DataValue::new_now(new_value)));
                }
            }

            if !updates.is_empty() {
                let update_count = updates.len();
                manager_for_update.set_values(&subs_for_update, updates.into_iter()).ok();
                update_counter_clone.fetch_add(update_count, Ordering::Relaxed);
            }
        }
    });

    // Handle write requests by setting variables as writable in address space
    // The actual write handling is done by setting writable flag on variables

    let handle_c = handle.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            eprintln!("Ctrl-C error: {}", e);
        }
        running_for_ctrlc.store(false, Ordering::Relaxed);
        handle_c.cancel();
    });

    let api_tags = tags_shared.clone();
    let api_manager = manager.clone();
    let api_ns = ns;

    // HTTP API for tag management
    std::thread::spawn(move || {
        let server = tiny_http::Server::http("127.0.0.1:8080").unwrap();
        println!("[API] HTTP API running on http://127.0.0.1:8080");

        for mut request in server.incoming_requests() {
            let url = request.url().to_string();
            let method = request.method().to_string();

            if method == "POST" && url == "/api/addTag" {
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).ok();

                let tag_name = parse_json_string(&body, "tagName").unwrap_or_else(|| "NewTag".to_string());
                let init_value = parse_json_number(&body, "value").unwrap_or_else(|| rand::random::<f64>() * 100.0);

                let mut tags = api_tags.write();
                let tag_idx = tags.len();
                let node_id = NodeId::new(api_ns, format!("Tag_{}", tag_idx));

                let new_tag = TagInfo {
                    node_id: node_id.clone(),
                    current_value: init_value,
                    display_name: tag_name.clone(),
                    description: format!("Dynamic tag {}", tag_idx),
                    unit: "".to_string(),
                    readonly: false, // New tags are writable by default
                    changerate: 0.0, // No auto-change for new tags
                };

                tags.push(new_tag);

                {
                    let address_space = api_manager.address_space();
                    let mut address_space = address_space.write();
                    let folder_id = NodeId::new(api_ns, "ReadWriteSimulationFolder");
                    let mut var = Variable::new(&node_id, &tag_name, &format!("Dynamic tag {}", tag_idx), init_value);
                    let access_level = make_access_level(true, true);
                    var.set_access_level(access_level);
                    var.set_user_access_level(access_level);
                    let _ = address_space.add_variables(vec![var], &folder_id);
                }

                let resp_body = format!("{{\"success\":true,\"nodeId\":\"ns={};s=Tag_{}\",\"idx\":{},\"value\":{}}}",
                    api_ns, tag_idx, tag_idx, init_value);
                let response = tiny_http::Response::new(
                    200.into(),
                    vec![tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()],
                    Cursor::new(resp_body),
                    None,
                    None
                );
                request.respond(response).ok();
            } else if method == "POST" && url == "/api/deleteTag" {
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).ok();

                let tag_idx = parse_json_number(&body, "idx").unwrap_or(0.0) as usize;
                let mut tags = api_tags.write();

                if tag_idx < tags.len() {
                    tags.remove(tag_idx);
                    let response = tiny_http::Response::new(
                        200.into(),
                        vec![tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()],
                        Cursor::new(r#"{"success":true}"#.to_string()),
                        None,
                        None
                    );
                    request.respond(response).ok();
                } else {
                    let response = tiny_http::Response::new(
                        400.into(),
                        vec![tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()],
                        Cursor::new(r#"{"success":false,"error":"Index out of range"}"#.to_string()),
                        None,
                        None
                    );
                    request.respond(response).ok();
                }
            } else {
                let response = tiny_http::Response::new(
                    404.into(),
                    vec![],
                    Cursor::new("Not found".to_string()),
                    None,
                    None
                );
                request.respond(response).ok();
            }
        }
    });

    println!();
    println!("ReadWrite Server running at opc.tcp://0.0.0.0:4840");
    println!("Press Ctrl+C to stop");

    server.run().await.unwrap();
}

fn parse_json_string(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    if let Some(pos) = json.find(&search) {
        let after = &json[pos + search.len()..];
        if let Some(colon) = after.find(':') {
            let value_part = after[colon + 1..].trim_start();
            if value_part.starts_with('"') {
                if let Some(end) = value_part[1..].find('"') {
                    return Some(value_part[1..1 + end].to_string());
                }
            }
        }
    }
    None
}

fn parse_json_number(json: &str, key: &str) -> Option<f64> {
    let search = format!("\"{}\"", key);
    if let Some(pos) = json.find(&search) {
        let after = &json[pos + search.len()..];
        if let Some(colon) = after.find(':') {
            let value_part = after[colon + 1..].trim_start();
            let end = value_part.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != 'e' && c != 'E' && c != '+').unwrap_or(value_part.len());
            if let Ok(n) = value_part[..end].trim().parse::<f64>() {
                return Some(n);
            }
        }
    }
    None
}