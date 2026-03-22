use openhanse_core::{
    CommunicationModeEnum, GatewayRuntimeConfig, HubRuntimeConfig, PeerModeEnum,
    PeerRuntimeConfig, PeerRuntimeHandle, current_unix_ms, default_port_for_peer,
    default_runtime_storage_dir, display_name_for_peer,
};
use openhanse_core::model::connect_model::ConnectDecisionModel;
use serde::Serialize;
use std::{
    env, fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    process,
    time::Duration,
};
use tokio::io::{self, AsyncBufReadExt, BufReader};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_SERVER_BASE_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 10;
const DEFAULT_COMMUNICATION_MODE: &str = "auto";

#[derive(Clone)]
struct ChatConfig {
    runtime: PeerRuntimeConfig,
    inbox_file: PathBuf,
}

#[tokio::main]
async fn main() {
    if let Err(message) = run(env::args().skip(1).collect()).await {
        eprintln!("Error: {message}");
        process::exit(1);
    }
}

async fn run(args: Vec<String>) -> Result<(), String> {
    if args
        .iter()
        .any(|value| matches!(value.as_str(), "--help" | "-h" | "help"))
    {
        print_usage();
        return Ok(());
    }

    let config = parse_chat_config(&args)?;
    if config.runtime.peer_mode.runs_gateway() {
        run_chat(config).await
    } else {
        run_hub(config.runtime).await
    }
}

async fn run_chat(config: ChatConfig) -> Result<(), String> {
    let runtime = PeerRuntimeHandle::start(config.runtime.clone()).await?;
    let info = runtime.info();
    let gateway_runtime = runtime
        .gateway_runtime()
        .ok_or_else(|| "peer mode does not enable gateway functionality".to_string())?;
    let gateway_info = info
        .gateway
        .ok_or_else(|| "gateway runtime info is unavailable".to_string())?;

    println!(
        "Starting {}",
        config
            .runtime
            .gateway
            .display_name
            .as_deref()
            .unwrap_or(&config.runtime.gateway.peer_id)
    );
    println!("Peer mode: {}", config.runtime.peer_mode.as_str());
    println!("Server: {}", gateway_info.server_base_url);
    println!(
        "Communication mode: {}",
        config.runtime.gateway.communication_mode.as_str()
    );
    println!(
        "Direct TCP endpoint: {}{}",
        gateway_info.direct_base_url, gateway_info.message_endpoint
    );
    if let Some(hub_info) = info.hub {
        println!("Hub HTTP bind: {}", hub_info.bind_addr);
        println!("Hub UDP discovery bind: {}", hub_info.discovery_udp_bind_addr);
    }
    if config.runtime.gateway.communication_mode.supports_direct_advertisement()
        && config.runtime.gateway.direct_bind_host == DEFAULT_HOST
    {
        println!(
            "Warning: advertising {}. This only works for same-device tests. Use --host <your LAN IP> for phone or cross-device messaging.",
            DEFAULT_HOST
        );
    }
    println!(
        "{} registered and ready.",
        config
            .runtime
            .gateway
            .display_name
            .as_deref()
            .unwrap_or(&config.runtime.gateway.peer_id)
    );

    let inbox_task = spawn_event_printer(gateway_runtime.subscribe_events());

    println!(
        "Type a message and press Enter to send it to {}.",
        config.runtime.gateway.target_peer_id
    );
    println!("Commands: /lookup, /connect, /inbox, /status, /quit");

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

    loop {
        print_prompt()?;
        let Some(input) = lines.next_line().await.map_err(|error| error.to_string())? else {
            break;
        };

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        match trimmed {
            "/quit" | "/exit" => break,
            "/lookup" => {
                let response = gateway_runtime.lookup_target().await?;
                print_json(&response)?;
            }
            "/connect" => {
                let response = gateway_runtime.connect_target().await?;
                print_json(&connect_summary(&response))?;
            }
            "/inbox" => print_inbox(&config.inbox_file)?,
            "/status" => print_json(&runtime.status().await)?,
            message => {
                let response = gateway_runtime.send_message(message.to_string()).await?;
                print_json(&response)?;
            }
        }
    }

    inbox_task.abort();
    runtime.stop().await?;
    Ok(())
}

async fn run_hub(config: PeerRuntimeConfig) -> Result<(), String> {
    let runtime = PeerRuntimeHandle::start(config).await?;
    let info = runtime.info();
    let hub_info = info
        .hub
        .ok_or_else(|| "hub runtime info is unavailable".to_string())?;
    println!("Peer mode: {}", info.peer_mode);
    println!("Hub HTTP bind: {}", hub_info.bind_addr);
    println!("Hub UDP discovery bind: {}", hub_info.discovery_udp_bind_addr);
    println!("Hub is running. Press Ctrl-C to stop.");
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("failed to wait for Ctrl-C: {error}"))?;
    runtime.stop().await
}

fn parse_chat_config(args: &[String]) -> Result<ChatConfig, String> {
    let peer_id = required_option(args, "--id")?;
    let peer_mode =
        parse_peer_mode(&optional_option(args, "--peer-mode").unwrap_or_else(|| "gateway".to_string()))?;
    let target_peer_id = match optional_option(args, "--target") {
        Some(target_peer_id) => target_peer_id,
        None if peer_mode.runs_gateway() => {
            return Err("--target is required unless --peer-mode hub is used".to_string());
        }
        None => peer_id.clone(),
    };
    let host = optional_option(args, "--host").unwrap_or_else(default_host);
    let port = optional_option(args, "--port")
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| format!("invalid --port value: {value}"))
        })
        .transpose()?
        .unwrap_or_else(|| default_port_for_peer(&peer_id));
    let server_base_url = optional_option(args, "--server")
        .unwrap_or_else(|| DEFAULT_SERVER_BASE_URL.to_string());
    let communication_mode = optional_option(args, "--communication-mode")
        .unwrap_or_else(|| DEFAULT_COMMUNICATION_MODE.to_string());
    let heartbeat_interval_secs = optional_option(args, "--heartbeat-interval-secs")
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("invalid --heartbeat-interval-secs value: {value}"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECS);
    let device_key =
        optional_option(args, "--device-key").unwrap_or_else(|| format!("device-key-{peer_id}"));
    let display_name =
        optional_option(args, "--display-name").or_else(|| Some(display_name_for_peer(&peer_id)));
    let runtime_root = runtime_root();
    let storage_dir = default_runtime_storage_dir(&runtime_root, &peer_id);
    let inbox_file = storage_dir.join("inbox.jsonl");

    ensure_parent_dir(&inbox_file)?;

    Ok(ChatConfig {
        runtime: PeerRuntimeConfig {
            peer_mode,
            gateway: GatewayRuntimeConfig {
                peer_id: peer_id.clone(),
                device_key,
                display_name,
                target_peer_id,
                server_base_url: server_base_url.clone(),
                direct_bind_host: host,
                direct_bind_port: port,
                communication_mode: parse_communication_mode(&communication_mode)?,
                heartbeat_interval_secs,
                storage_dir,
            },
            hub: hub_runtime_config(&server_base_url)?,
        },
        inbox_file,
    })
}

fn parse_communication_mode(value: &str) -> Result<CommunicationModeEnum, String> {
    match value {
        "auto" => Ok(CommunicationModeEnum::Auto),
        "direct" => Ok(CommunicationModeEnum::Direct),
        "relay" => Ok(CommunicationModeEnum::RelayOnly),
        _ => Err(format!(
            "invalid --communication-mode value: {value} (expected 'auto', 'direct', or 'relay')"
        )),
    }
}

fn parse_peer_mode(value: &str) -> Result<PeerModeEnum, String> {
    match value {
        "both" => Ok(PeerModeEnum::Both),
        "hub" => Ok(PeerModeEnum::Hub),
        "gateway" => Ok(PeerModeEnum::Gateway),
        _ => Err(format!(
            "invalid --peer-mode value: {value} (expected 'both', 'hub', or 'gateway')"
        )),
    }
}

fn connect_summary(decision: &ConnectDecisionModel) -> serde_json::Value {
    match decision {
        ConnectDecisionModel::Direct { direct } => serde_json::json!({
            "mode": "direct",
            "peer_id": direct.peer_id,
            "reachability_candidates": direct.reachability_candidates,
            "message_endpoint": direct.message_endpoint,
            "decision_reason": direct.decision_reason,
        }),
        ConnectDecisionModel::Relay { relay } => serde_json::json!({
            "mode": "relay",
            "relay_session_id": relay.relay_session_id,
            "source_peer_id": relay.source_peer_id,
            "target_peer_id": relay.target_peer_id,
            "expires_at_unix_ms": relay.expires_at_unix_ms,
            "decision_reason": relay.decision_reason,
        }),
    }
}

fn required_option(args: &[String], name: &str) -> Result<String, String> {
    optional_option(args, name).ok_or_else(|| format!("{name} is required"))
}

fn optional_option(args: &[String], name: &str) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == name {
            return args.get(index + 1).cloned();
        }
        index += 1;
    }
    None
}

fn print_prompt() -> Result<(), String> {
    use std::io::Write;

    print!("> ");
    std::io::stdout().flush().map_err(|error| error.to_string())
}

fn default_host() -> String {
    detect_ipv4_for_outbound_traffic()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| DEFAULT_HOST.to_string())
}

fn detect_ipv4_for_outbound_traffic() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket
        .connect(SocketAddr::from((Ipv4Addr::new(1, 1, 1, 1), 80)))
        .ok()?;

    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

fn print_inbox(inbox_file: &Path) -> Result<(), String> {
    if !inbox_file.exists() {
        return Err("no inbox found yet for this peer".to_string());
    }

    let contents = fs::read_to_string(inbox_file).map_err(|error| error.to_string())?;
    let entries: Vec<serde_json::Value> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect::<Result<_, _>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&entries).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn runtime_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime")
}

fn hub_runtime_config(server_base_url: &str) -> Result<HubRuntimeConfig, String> {
    let (host, port) = host_and_port_from_url(server_base_url)?;
    let bind_host = if host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") {
        host
    } else {
        "0.0.0.0".to_string()
    };
    Ok(HubRuntimeConfig {
        bind_addr: format!("{bind_host}:{port}"),
        discovery_udp_bind_addr: format!("{bind_host}:3478"),
        ..HubRuntimeConfig::default()
    })
}

fn host_and_port_from_url(url: &str) -> Result<(String, u16), String> {
    let without_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    let authority = without_scheme
        .split('/')
        .next()
        .ok_or_else(|| format!("invalid server URL: {url}"))?;
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| format!("server URL must include a port: {url}"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("invalid server port in URL: {url}"))?;
    Ok((host.to_string(), port))
}

fn spawn_event_printer(
    mut events: tokio::sync::broadcast::Receiver<openhanse_core::UiEventModel>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event.kind == "message_received" {
                        println!();
                        println!("[inbox @ {}] {}", event.created_at_unix_ms, event.message);
                        let _ = print_prompt();
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
}

fn print_usage() {
    println!(
        "\
Usage:
  cargo run -- --id <peer_id> [--target <peer_id>] [--peer-mode <gateway|hub|both>] [--server <url>] [--host <host>] [--port <port>] [--communication-mode <auto|direct|relay>] [--display-name <name>] [--device-key <key>] [--heartbeat-interval-secs <seconds>]

Example:
  cargo run -- --id gateway-a --target gateway-b
  cargo run -- --id hub-a --peer-mode hub --server http://0.0.0.0:8080
  cargo run -- --id peer-a --target peer-b --peer-mode both --server http://192.168.1.10:8080
  cargo run -- --id gateway-b --target gateway-a
  cargo run -- --id gateway-a --target gateway-b --server http://1.2.3.4:8080 --communication-mode relay

Commands:
  Type any text and press Enter to send it.
  /lookup
  /connect
  /inbox
  /status
  /quit
"
    );
}

#[allow(dead_code)]
fn _unused_timestamp_for_future_cli_logging() -> u64 {
    current_unix_ms()
}
