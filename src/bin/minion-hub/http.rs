//! Private HTTP API for managing hub machines.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use crate::command::{CommandRunner, SystemCommandRunner};
use crate::json::{json_escape, parse_json_object};
use crate::paths::HubPaths;
use crate::reconcile::apply_runtime_changes;
use crate::store::{HubStore, Machine, MachinePatch};
use crate::HUB_VPN_IP;

pub(crate) fn serve(paths: HubPaths, listen: SocketAddr) -> Result<()> {
    if listen.ip().is_unspecified() {
        bail!("minion-hub serve must not listen on an unspecified address");
    }
    if listen.ip() != HUB_VPN_IP {
        bail!(
            "minion-hub serve must listen on private WireGuard address {}",
            HUB_VPN_IP
        );
    }

    let listener =
        TcpListener::bind(listen).with_context(|| format!("failed to bind {}", listen))?;
    let runner = SystemCommandRunner::new(false);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = match read_http_request(&mut stream) {
                    Ok(request) => route_request(&paths, &runner, request),
                    Err(error) => HttpResponse::json_error(400, &error.to_string()),
                };
                let _ = stream.write_all(&response.to_bytes());
            }
            Err(error) => eprintln!("failed to accept connection: {}", error),
        }
    }

    Ok(())
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: String,
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

impl HttpResponse {
    fn json(status: u16, body: String) -> Self {
        Self { status, body }
    }

    fn json_error(status: u16, message: &str) -> Self {
        Self {
            status,
            body: format!("{{\"error\":\"{}\"}}", json_escape(message)),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            _ => "Internal Server Error",
        };
        let body = if self.status == 204 {
            String::new()
        } else {
            self.body.clone()
        };
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            reason,
            body.len(),
            body
        )
        .into_bytes()
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > 16 * 1024 {
            bail!("HTTP headers are too large");
        }
    }

    let header_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
        .context("HTTP request is missing header terminator")?;

    let header = String::from_utf8(buffer[..header_end].to_vec())?;
    let mut lines = header.split("\r\n");
    let request_line = lines
        .next()
        .context("HTTP request is missing request line")?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .context("HTTP request is missing method")?
        .to_string();
    let path = request_parts
        .next()
        .context("HTTP request is missing path")?
        .to_string();

    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    if content_length > 64 * 1024 {
        bail!("HTTP body is too large");
    }

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        body: String::from_utf8(body)?,
    })
}

fn route_request(
    paths: &HubPaths,
    runner: &dyn CommandRunner,
    request: HttpRequest,
) -> HttpResponse {
    let path = request.path.split('?').next().unwrap_or(&request.path);
    let store = HubStore::new(paths.clone());

    let result = match (request.method.as_str(), path) {
        ("GET", "/machines") => store
            .list_machines()
            .map(|machines| HttpResponse::json(200, machines_json(&machines))),
        ("POST", "/machines") => parse_machine_body(&request.body)
            .and_then(|machine| store.add_machine(machine))
            .and_then(|machine| {
                apply_runtime_changes(paths, runner)?;
                Ok(HttpResponse::json(201, machine_json(&machine)))
            }),
        _ if path.starts_with("/machines/") => {
            let name = &path["/machines/".len()..];
            route_machine_request(&store, paths, runner, &request, name)
        }
        _ => Ok(HttpResponse::json_error(404, "not found")),
    };

    result.unwrap_or_else(error_response)
}

fn route_machine_request(
    store: &HubStore,
    paths: &HubPaths,
    runner: &dyn CommandRunner,
    request: &HttpRequest,
    name: &str,
) -> Result<HttpResponse> {
    match request.method.as_str() {
        "GET" => store
            .get_machine(name)
            .map(|machine| HttpResponse::json(200, machine_json(&machine))),
        "PATCH" => parse_machine_patch_body(&request.body)
            .and_then(|patch| store.patch_machine(name, patch))
            .and_then(|machine| {
                apply_runtime_changes(paths, runner)?;
                Ok(HttpResponse::json(200, machine_json(&machine)))
            }),
        "DELETE" => store.delete_machine(name).and_then(|_| {
            apply_runtime_changes(paths, runner)?;
            Ok(HttpResponse::json(204, String::new()))
        }),
        _ => Ok(HttpResponse::json_error(405, "method not allowed")),
    }
}

fn error_response(error: anyhow::Error) -> HttpResponse {
    let message = error.to_string();
    // Store errors are still anyhow strings; keep these wording checks in sync
    // with store.rs until they become typed errors.
    if message.contains("not found") {
        HttpResponse::json_error(404, &message)
    } else if message.contains("already exists") || message.contains("already assigned") {
        HttpResponse::json_error(409, &message)
    } else {
        HttpResponse::json_error(400, &message)
    }
}

fn parse_machine_body(body: &str) -> Result<Machine> {
    let object = parse_json_object(body)?;
    let name = required_json_string(&object, "name")?;
    let vpn_ip = required_json_string(&object, "vpn_ip")?.parse::<Ipv4Addr>()?;
    let public_key = required_json_string(&object, "public_key")?;
    Ok(Machine {
        name,
        vpn_ip,
        public_key,
    })
}

fn parse_machine_patch_body(body: &str) -> Result<MachinePatch> {
    let object = parse_json_object(body)?;
    let vpn_ip = object
        .get("vpn_ip")
        .map(|value| value.parse::<Ipv4Addr>())
        .transpose()?;
    Ok(MachinePatch {
        name: object.get("name").cloned(),
        vpn_ip,
        public_key: object.get("public_key").cloned(),
    })
}

fn required_json_string(object: &BTreeMap<String, String>, key: &str) -> Result<String> {
    object
        .get(key)
        .cloned()
        .ok_or_else(|| anyhow!("request body is missing {}", key))
}

fn machines_json(machines: &[Machine]) -> String {
    format!(
        "[{}]",
        machines
            .iter()
            .map(machine_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn machine_json(machine: &Machine) -> String {
    format!(
        "{{\"name\":\"{}\",\"vpn_ip\":\"{}\",\"public_key\":\"{}\"}}",
        json_escape(&machine.name),
        machine.vpn_ip,
        json_escape(&machine.public_key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{prepared_store, PEER_KEY, PEER_KEY_2};
    use std::fs;

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            body: body.to_string(),
        }
    }

    #[test]
    fn api_crud_updates_wireguard_and_coredns_files() {
        let (_dir, paths, runner) = prepared_store();

        let created = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"web-01\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(created.status, 201);
        assert!(fs::read_to_string(&paths.wg_config)
            .unwrap()
            .contains("AllowedIPs = 10.42.42.2/32"));
        assert_eq!(
            fs::read_to_string(&paths.coredns_hosts).unwrap(),
            "10.42.42.2 web-01\n"
        );

        let listed = route_request(&paths, &runner, request("GET", "/machines", ""));
        assert_eq!(listed.status, 200);
        assert!(listed.body.contains("\"name\":\"web-01\""));

        let patched = route_request(
            &paths,
            &runner,
            request(
                "PATCH",
                "/machines/web-01",
                &format!(
                    "{{\"name\":\"web_02\",\"vpn_ip\":\"10.42.42.3\",\"public_key\":\"{}\"}}",
                    PEER_KEY_2
                ),
            ),
        );
        assert_eq!(patched.status, 200);
        assert!(fs::read_to_string(&paths.wg_config)
            .unwrap()
            .contains("AllowedIPs = 10.42.42.3/32"));
        assert_eq!(
            fs::read_to_string(&paths.coredns_hosts).unwrap(),
            "10.42.42.3 web_02\n"
        );

        let deleted = route_request(&paths, &runner, request("DELETE", "/machines/web_02", ""));
        assert_eq!(deleted.status, 204);
        assert!(!fs::read_to_string(&paths.wg_config)
            .unwrap()
            .contains("[Peer]"));
        assert_eq!(fs::read_to_string(&paths.coredns_hosts).unwrap(), "");
    }

    #[test]
    fn api_rejects_invalid_input_and_duplicate_ips() {
        let (_dir, paths, runner) = prepared_store();

        let invalid_name = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"bad;name\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(invalid_name.status, 400);

        let invalid_ip = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"web\",\"vpn_ip\":\"10.42.42.1\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(invalid_ip.status, 400);

        let invalid_key = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                "{\"name\":\"web\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"bad\"}",
            ),
        );
        assert_eq!(invalid_key.status, 400);

        let first = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"web\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(first.status, 201);

        let duplicate_ip = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"db\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY_2
                ),
            ),
        );
        assert_eq!(duplicate_ip.status, 409);
    }
}
