//! Explicit NetworkManager actions for existing VPN/WireGuard profiles.
//! No credential import, privilege escalation, autoconnect, or startup probes.
use crate::process::{self, ProcessSpec};
use anyhow::{Result, ensure};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, Serialize)]
pub struct Connection {
    pub name: String,
    pub uuid: String,
    pub kind: String,
    pub active: bool,
}
fn fields(line: &str) -> Vec<String> {
    let mut values = vec![String::new()];
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            values.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ':' {
            values.push(String::new());
        } else {
            values.last_mut().unwrap().push(c);
        }
    }
    values
}
fn parse(lines: &[String]) -> Vec<Connection> {
    lines
        .iter()
        .filter_map(|line| {
            let columns = fields(line);
            if columns.len() != 4
                || !["vpn", "wireguard"].contains(&columns[2].as_str())
                || uuid::Uuid::parse_str(&columns[1]).is_err()
            {
                return None;
            }
            Some(Connection {
                name: ytui_core::clean_text(&columns[0]),
                uuid: columns[1].clone(),
                kind: columns[2].clone(),
                active: columns[3] == "yes",
            })
        })
        .collect()
}
fn spec(program: &str, args: Vec<String>) -> ProcessSpec {
    let mut spec = ProcessSpec::new(program, args, 40);
    spec.env.push(("LC_ALL".into(), "C".into()));
    spec.max_output_bytes = 128 * 1024;
    spec
}
pub async fn list(program: &str, cancel: CancellationToken) -> Result<Vec<Connection>> {
    let result = process::run(
        spec(
            program,
            [
                "--terse",
                "--escape",
                "yes",
                "--fields",
                "NAME,UUID,TYPE,ACTIVE",
                "connection",
                "show",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ),
        cancel,
        |_| {},
    )
    .await?;
    ensure!(
        result.success,
        "NetworkManager is unavailable: {}",
        process::diagnostic(&result.stderr)
    );
    Ok(parse(&result.lines))
}
pub async fn change(
    program: &str,
    uuid: &str,
    connect: bool,
    cancel: CancellationToken,
) -> Result<()> {
    uuid::Uuid::parse_str(uuid)?;
    // Refuse IDs belonging to Ethernet/Wi-Fi connections; verify immediately before mutation.
    ensure!(
        list(program, cancel.clone())
            .await?
            .iter()
            .any(|c| c.uuid == uuid),
        "Select a listed VPN/WireGuard UUID"
    );
    let result = process::run(
        spec(
            program,
            [
                "--wait",
                "30",
                "connection",
                if connect { "up" } else { "down" },
                "uuid",
                uuid,
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ),
        cancel,
        |_| {},
    )
    .await?;
    ensure!(
        result.success,
        "VPN action failed: {}",
        process::diagnostic(&result.stderr)
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_explicit_vpn_ids_can_change_and_names_are_unescaped() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("nmcli");
        std::fs::write(&program, "#!/bin/sh\nif [ \"$1\" = --terse ]; then printf '%s\\n' 'Proton\\: Test:11111111-1111-4111-8111-111111111111:wireguard:no' 'Home:22222222-2222-4222-8222-222222222222:802-3-ethernet:yes'; exit 0; fi\n[ \"$4\" = up ] && [ \"$6\" = 11111111-1111-4111-8111-111111111111 ]\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let program = program.to_str().unwrap();
        let connections = list(program, CancellationToken::new()).await.unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].name, "Proton: Test");
        change(
            program,
            &connections[0].uuid,
            true,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(
            change(
                program,
                "22222222-2222-4222-8222-222222222222",
                true,
                CancellationToken::new()
            )
            .await
            .is_err()
        );
        assert!(
            change(program, "--anything", true, CancellationToken::new())
                .await
                .is_err()
        );
    }
}
